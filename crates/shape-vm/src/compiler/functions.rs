//! Function and closure compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use shape_ast::ast::{
    DestructurePattern, Expr, FunctionDef, Literal, ObjectEntry, Span, Statement, VarKind,
    VariableDecl,
};
use shape_ast::error::{ErrorNote, Result, ShapeError};
use shape_runtime::type_schema::FieldType;
use shape_value::ValueWord;
use std::collections::{HashMap, HashSet};

use super::{BytecodeCompiler, ParamPassMode};

/// Display a type annotation using C-ABI convention (Vec instead of Array).
fn cabi_type_display(ann: &shape_ast::ast::TypeAnnotation) -> String {
    match ann {
        shape_ast::ast::TypeAnnotation::Array(inner) => {
            format!("Vec<{}>", cabi_type_display(inner))
        }
        other => other.to_type_string(),
    }
}

impl BytecodeCompiler {
    pub(super) fn explicit_param_pass_modes(
        params: &[shape_ast::ast::FunctionParameter],
    ) -> Vec<ParamPassMode> {
        params
            .iter()
            .map(|param| {
                if param.is_mut_reference {
                    ParamPassMode::ByRefExclusive
                } else if param.is_reference {
                    ParamPassMode::ByRefShared
                } else {
                    ParamPassMode::ByValue
                }
            })
            .collect()
    }

    pub(super) fn effective_function_like_pass_modes(
        &self,
        name: Option<&str>,
        params: &[shape_ast::ast::FunctionParameter],
        body: Option<&[shape_ast::ast::Statement]>,
    ) -> Vec<ParamPassMode> {
        if let Some(name) = name {
            if let Some(inferred_modes) = self.inferred_param_pass_modes.get(name) {
                let fallback_modes = Self::explicit_param_pass_modes(params);
                return fallback_modes
                    .into_iter()
                    .enumerate()
                    .map(|(idx, fallback)| inferred_modes.get(idx).copied().unwrap_or(fallback))
                    .collect();
            }
            if let Some(func_idx) = self.find_function(name)
                && let Some(func) = self.program.functions.get(func_idx)
            {
                let fallback_modes = Self::explicit_param_pass_modes(params);
                let registered_modes =
                    Self::pass_modes_from_ref_flags(&func.ref_params, &func.ref_mutates);
                return fallback_modes
                    .into_iter()
                    .enumerate()
                    .map(|(idx, fallback)| registered_modes.get(idx).copied().unwrap_or(fallback))
                    .collect();
            }
        }

        let mut modes = Self::explicit_param_pass_modes(params);
        let Some(body) = body else {
            return modes;
        };

        let caller_ref_params: Vec<_> = modes.iter().map(|mode| mode.is_reference()).collect();
        if !caller_ref_params.iter().any(|is_ref| *is_ref) {
            return modes;
        }

        let mut known_callable_modes: HashMap<String, Vec<ParamPassMode>> = self
            .program
            .functions
            .iter()
            .map(|func| {
                (
                    func.name.clone(),
                    Self::pass_modes_from_ref_flags(&func.ref_params, &func.ref_mutates),
                )
            })
            .collect();
        for scope in &self.locals {
            for (binding_name, local_idx) in scope {
                if let Some(pass_modes) = self.local_callable_pass_modes.get(local_idx) {
                    known_callable_modes.insert(binding_name.clone(), pass_modes.clone());
                }
            }
        }
        for (binding_name, binding_idx) in &self.module_bindings {
            if let Some(pass_modes) = self.module_binding_callable_pass_modes.get(binding_idx) {
                known_callable_modes.insert(binding_name.clone(), pass_modes.clone());
            }
        }

        let callee_ref_params: HashMap<String, Vec<bool>> = known_callable_modes
            .iter()
            .map(|(callee_name, pass_modes)| {
                (
                    callee_name.clone(),
                    pass_modes.iter().map(|mode| mode.is_reference()).collect(),
                )
            })
            .collect();
        let caller_name = name.unwrap_or("__function_expr__");
        let mut direct_mutates = vec![false; params.len()];
        let mut edges = Vec::new();
        let mut param_index_by_name = HashMap::new();
        for (idx, param) in params.iter().enumerate() {
            for param_name in param.get_identifiers() {
                param_index_by_name.insert(param_name, idx);
            }
        }
        for stmt in body {
            Self::analyze_statement_for_ref_mutation(
                stmt,
                caller_name,
                &param_index_by_name,
                &caller_ref_params,
                &callee_ref_params,
                &mut direct_mutates,
                &mut edges,
            );
        }
        for (_, caller_idx, callee_name, callee_idx) in edges {
            if known_callable_modes
                .get(&callee_name)
                .and_then(|modes| modes.get(callee_idx))
                .is_some_and(|mode| mode.is_exclusive())
                && let Some(flag) = direct_mutates.get_mut(caller_idx)
            {
                *flag = true;
            }
        }
        for (idx, direct_mutates) in direct_mutates.into_iter().enumerate() {
            if direct_mutates && modes.get(idx).is_some_and(|mode| mode.is_reference()) {
                modes[idx] = ParamPassMode::ByRefExclusive;
            }
        }

        modes
    }

    pub(super) fn compile_function(&mut self, func_def: &FunctionDef) -> Result<()> {
        // Validate annotation target kinds before compilation
        self.validate_annotation_targets(func_def)?;

        let mut effective_def = func_def.clone();
        let effective_pass_modes = self.effective_function_like_pass_modes(
            Some(&effective_def.name),
            &effective_def.params,
            Some(&effective_def.body),
        );
        for (idx, param) in effective_def.params.iter_mut().enumerate() {
            let effective_mode = effective_pass_modes
                .get(idx)
                .copied()
                .unwrap_or(ParamPassMode::ByValue);
            if param.type_annotation.is_none()
                && param.simple_name().is_some()
                && effective_mode.is_reference()
            {
                param.is_reference = true;
            }
            if effective_mode.is_exclusive() {
                param.is_mut_reference = true;
            }
        }
        let has_const_template_params = effective_def.params.iter().any(|p| p.is_const);
        let has_specialization_bindings = self
            .specialization_const_bindings
            .contains_key(&effective_def.name);

        // Execute comptime annotation handlers before compilation.
        // These run at compile time and can inspect/modify the target.
        // Template bases (functions with const parameters) skip comptime handler
        // execution until a concrete call-site specialization binds those consts.
        if !(has_const_template_params && !has_specialization_bindings)
            && self.execute_comptime_handlers(&mut effective_def)?
        {
            self.function_defs.remove(&effective_def.name);
            return Ok(());
        }

        // Keep the registry synchronized with the final mutated function shape.
        // This is used by expansion/inspection tooling.
        self.function_defs
            .insert(effective_def.name.clone(), effective_def.clone());

        // Lower every compiled function to MIR and run the shared borrow analysis.
        // Diagnostics still come from the bytecode compiler paths for now, but
        // this gives real compilations a single analysis artifact we can
        // progressively hook into later.
        let mir_lowering = crate::mir::lowering::lower_function_detailed(
            &effective_def.name,
            &effective_def.params,
            &effective_def.body,
            effective_def.name_span,
        );
        let mut mir_analysis = crate::mir::solver::analyze(&mir_lowering.mir);
        crate::mir::repair::attach_repairs(&mut mir_analysis, &mir_lowering.mir);
        let first_mir_error = if mir_lowering.had_fallbacks {
            None
        } else {
            mir_analysis.errors.first().cloned()
        };
        self.mir_functions
            .insert(effective_def.name.clone(), mir_lowering.mir);
        self.mir_borrow_analyses
            .insert(effective_def.name.clone(), mir_analysis);
        if let Some(error) = first_mir_error.as_ref() {
            return Err(self.mir_borrow_error(error));
        }

        // Track whether __original__ alias is active so we can clean it up.
        let has_original_alias = self.function_aliases.contains_key("__original__");

        // Enable __* builtin access only for stdlib-origin functions
        // (or preserve if an enclosing stdlib compilation context already enabled it).
        let saved_allow_internal = self.allow_internal_builtins;
        self.allow_internal_builtins = saved_allow_internal
            || effective_def
                .declaring_module_path
                .as_deref()
                .is_some_and(|module_path| module_path.starts_with("std::"));

        // Check for annotation-based wrapping BEFORE compiling
        let annotations = self.find_compiled_annotations(&effective_def);
        if annotations.len() == 1 {
            self.compile_wrapped_function(
                &effective_def,
                annotations.into_iter().next().expect("checked len == 1"),
            )?;
        } else if annotations.len() > 1 {
            self.compile_chained_annotations(&effective_def, annotations)?;
        } else {
            self.compile_function_body(&effective_def)?;
        }

        // Restore previous flag (safe for nested compilation).
        self.allow_internal_builtins = saved_allow_internal;

        // Clean up __original__ alias after the replacement body is compiled.
        if has_original_alias {
            self.function_aliases.remove("__original__");
        }

        // Runtime lifecycle hooks (`on_define`, `metadata`) are invoked at
        // definition time by emitting top-level calls after function compilation.
        self.emit_annotation_lifecycle_calls(&effective_def)
    }

    fn mir_borrow_error_message(
        &self,
        kind: crate::mir::analysis::BorrowErrorKind,
    ) -> (&'static str, &'static str) {
        match kind {
            crate::mir::analysis::BorrowErrorKind::ConflictSharedExclusive => (
                "[B0001] cannot mutably borrow this value while shared borrows are active",
                "move the mutable borrow later, or end the shared borrow sooner",
            ),
            crate::mir::analysis::BorrowErrorKind::ConflictExclusiveExclusive => (
                "[B0001] cannot mutably borrow this value because it is already borrowed",
                "end the previous mutable borrow before creating another one",
            ),
            crate::mir::analysis::BorrowErrorKind::ReadWhileExclusivelyBorrowed => (
                "[B0001] cannot read this value while it is mutably borrowed",
                "read through the existing reference, or move the read after the borrow ends",
            ),
            crate::mir::analysis::BorrowErrorKind::WriteWhileBorrowed => (
                "[B0002] cannot write to this value while it is borrowed",
                "move this write after the borrow ends",
            ),
            crate::mir::analysis::BorrowErrorKind::ReferenceEscape => (
                "cannot return or store a reference that outlives its owner",
                "return an owned value instead of a reference",
            ),
            crate::mir::analysis::BorrowErrorKind::ReferenceStoredInArray => (
                "cannot store a reference in an array — references are scoped borrows that cannot escape into collections. Use owned values instead",
                "store owned values in the array instead of references",
            ),
            crate::mir::analysis::BorrowErrorKind::ReferenceStoredInObject => (
                "cannot store a reference in an object or struct literal — references are scoped borrows that cannot escape into aggregate values. Use owned values instead",
                "store owned values in the object or struct instead of references",
            ),
            crate::mir::analysis::BorrowErrorKind::ReferenceStoredInEnum => (
                "cannot store a reference in an enum payload — references are scoped borrows that cannot escape into aggregate values. Use owned values instead",
                "store owned values in the enum payload instead of references",
            ),
            crate::mir::analysis::BorrowErrorKind::ReferenceEscapeIntoClosure => (
                "[B0003] reference cannot escape into a closure",
                "capture an owned value instead of a reference",
            ),
            crate::mir::analysis::BorrowErrorKind::UseAfterMove => (
                "cannot use this value after it was moved",
                "clone the value before moving it, or stop using the original after the move",
            ),
            crate::mir::analysis::BorrowErrorKind::ExclusiveRefAcrossTaskBoundary => (
                "cannot move an exclusive reference across a task boundary",
                "keep the mutable reference within the current task or pass an owned value instead",
            ),
        }
    }

    fn mir_borrow_origin_note(&self, kind: crate::mir::analysis::BorrowErrorKind) -> &'static str {
        match kind {
            crate::mir::analysis::BorrowErrorKind::ConflictSharedExclusive
            | crate::mir::analysis::BorrowErrorKind::ConflictExclusiveExclusive
            | crate::mir::analysis::BorrowErrorKind::ReadWhileExclusivelyBorrowed
            | crate::mir::analysis::BorrowErrorKind::WriteWhileBorrowed => {
                "conflicting borrow originates here"
            }
            crate::mir::analysis::BorrowErrorKind::ReferenceEscape
            | crate::mir::analysis::BorrowErrorKind::ReferenceStoredInArray
            | crate::mir::analysis::BorrowErrorKind::ReferenceStoredInObject
            | crate::mir::analysis::BorrowErrorKind::ReferenceStoredInEnum
            | crate::mir::analysis::BorrowErrorKind::ReferenceEscapeIntoClosure => {
                "reference originates here"
            }
            crate::mir::analysis::BorrowErrorKind::UseAfterMove => "value was moved here",
            crate::mir::analysis::BorrowErrorKind::ExclusiveRefAcrossTaskBoundary => {
                "reference originates here"
            }
        }
    }

    fn mir_borrow_error(&self, error: &crate::mir::analysis::BorrowError) -> ShapeError {
        let (message, default_hint) = self.mir_borrow_error_message(error.kind.clone());
        let mut location = self.span_to_source_location(error.span);
        location.hints.push(default_hint.to_string());
        if let Some(repair) = error.repairs.first() {
            location.hints.push(repair.description.clone());
        }
        location.notes.push(ErrorNote {
            message: self.mir_borrow_origin_note(error.kind.clone()).to_string(),
            location: Some(self.span_to_source_location(error.loan_span)),
        });
        if let Some(last_use_span) = error.last_use_span {
            location.notes.push(ErrorNote {
                message: "borrow is still needed here".to_string(),
                location: Some(self.span_to_source_location(last_use_span)),
            });
        }
        ShapeError::SemanticError {
            message: message.to_string(),
            location: Some(location),
        }
    }

    pub(super) fn compile_foreign_function(
        &mut self,
        def: &shape_ast::ast::ForeignFunctionDef,
    ) -> Result<()> {
        // Validate `out` params: only allowed on extern C, must be ptr, no const/&/default.
        self.validate_out_params(def)?;

        // Foreign function bodies are opaque — require explicit type annotations.
        // Dynamic-language runtimes require Result<T> returns; native ABI
        // declarations (`extern "C"`) do not.
        let dynamic_language = !def.is_native_abi();
        let type_errors = def.validate_type_annotations(dynamic_language);
        if let Some((msg, span)) = type_errors.into_iter().next() {
            let loc = if span.is_dummy() {
                self.span_to_source_location(def.name_span)
            } else {
                self.span_to_source_location(span)
            };
            return Err(ShapeError::SemanticError {
                message: msg,
                location: Some(loc),
            });
        }
        if def.is_native_abi() && def.is_async {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "extern native function '{}' cannot be async (native ABI calls are synchronous)",
                    def.name
                ),
                location: Some(self.span_to_source_location(def.name_span)),
            });
        }

        // The function slot was already registered by register_item_functions.
        // Find its index.
        let func_idx = self
            .find_function(&def.name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Internal error: foreign function '{}' not registered",
                    def.name
                ),
                location: None,
            })?;

        // Determine out-param indices.
        let out_param_indices: Vec<usize> = def
            .params
            .iter()
            .enumerate()
            .filter(|(_, p)| p.is_out)
            .map(|(i, _)| i)
            .collect();
        let has_out_params = !out_param_indices.is_empty();
        let non_out_count = def.params.len() - out_param_indices.len();

        // Create the ForeignFunctionEntry
        let param_names: Vec<String> = def
            .params
            .iter()
            .flat_map(|p| p.get_identifiers())
            .collect();
        let param_types: Vec<String> = def
            .params
            .iter()
            .map(|p| {
                p.type_annotation
                    .as_ref()
                    .map(|t| t.to_type_string())
                    .unwrap_or_else(|| "any".to_string())
            })
            .collect();
        let return_type = def.return_type.as_ref().map(|t| t.to_type_string());
        let total_c_arg_count = def.params.len() as u16;

        let native_abi = if let Some(native) = &def.native_abi {
            let signature = self.build_native_c_signature(def)?;
            Some(crate::bytecode::NativeAbiSpec {
                abi: native.abi.clone(),
                library: self
                    .resolve_native_library_alias(&native.library, native.package_key.as_deref())?,
                symbol: native.symbol.clone(),
                signature,
            })
        } else {
            None
        };

        // Register an anonymous schema if the return type contains an inline object.
        let return_type_schema_id = if def.is_native_abi() {
            None
        } else {
            def.return_type
                .as_ref()
                .and_then(|ann| Self::find_object_in_annotation(ann))
                .map(|obj_fields| {
                    let schema_name = format!("__ffi_{}_return", def.name);
                    // Check if already registered (e.g. from a previous compilation pass)
                    let registry = self.type_tracker.schema_registry_mut();
                    if let Some(existing) = registry.get(&schema_name) {
                        return existing.id as u32;
                    }
                    let mut builder =
                        shape_runtime::type_schema::TypeSchemaBuilder::new(schema_name);
                    for f in obj_fields {
                        let field_type = Self::type_annotation_to_field_type(&f.type_annotation);
                        let anns: Vec<shape_runtime::type_schema::FieldAnnotation> = f
                            .annotations
                            .iter()
                            .map(|a| {
                                let args = a
                                    .args
                                    .iter()
                                    .filter_map(Self::eval_annotation_arg)
                                    .collect();
                                shape_runtime::type_schema::FieldAnnotation {
                                    name: a.name.clone(),
                                    args,
                                }
                            })
                            .collect();
                        builder = builder.field_with_meta(f.name.clone(), field_type, anns);
                    }
                    builder.register(registry) as u32
                })
                .or_else(|| {
                    // Try named type reference (e.g. Result<MyType>)
                    def.return_type
                        .as_ref()
                        .and_then(|ann| Self::find_reference_in_annotation(ann))
                        .and_then(|name| {
                            self.type_tracker
                                .schema_registry()
                                .get(name)
                                .map(|s| s.id as u32)
                        })
                })
        };

        let foreign_idx = self.program.foreign_functions.len() as u16;
        let mut entry = crate::bytecode::ForeignFunctionEntry {
            name: def.name.clone(),
            language: def.language.clone(),
            body_text: def.body_text.clone(),
            param_names: param_names.clone(),
            param_types,
            return_type,
            arg_count: total_c_arg_count,
            is_async: def.is_async,
            dynamic_errors: dynamic_language,
            return_type_schema_id,
            content_hash: None,
            native_abi,
        };
        entry.compute_content_hash();
        self.program.foreign_functions.push(entry);

        // Emit a jump over the function body so the VM doesn't fall through
        // into the stub instructions during top-level execution.
        let jump_over = self.emit_jump(OpCode::Jump, 0);

        // Build a dedicated blob for the extern stub so content-addressed
        // linking can resolve function-value constants without zero-hash deps.
        let saved_blob_builder = self.current_blob_builder.take();
        self.current_blob_builder = Some(super::FunctionBlobBuilder::new(
            def.name.clone(),
            self.program.current_offset(),
            self.program.constants.len(),
            self.program.strings.len(),
        ));

        // Record entry point of the stub function body
        let entry_point = self.program.instructions.len();

        if has_out_params {
            self.emit_out_param_stub(def, func_idx, foreign_idx, &out_param_indices)?;
        } else {
            // Simple stub: LoadLocal(0..N), PushConst(N), CallForeign, ReturnValue
            let arg_count = total_c_arg_count;
            for i in 0..arg_count {
                self.emit(Instruction::new(OpCode::LoadLocal, Some(Operand::Local(i))));
            }
            let arg_count_const = self
                .program
                .add_constant(Constant::Number(arg_count as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count_const)),
            ));
            self.emit(Instruction::new(
                OpCode::CallForeign,
                Some(Operand::ForeignFunction(foreign_idx)),
            ));
            self.emit(Instruction::simple(OpCode::ReturnValue));
        }

        // Update function metadata before finalizing blob.
        let caller_visible_arity = if has_out_params {
            non_out_count as u16
        } else {
            total_c_arg_count
        };
        let func = &mut self.program.functions[func_idx];
        func.entry_point = entry_point;
        func.arity = caller_visible_arity;
        if has_out_params {
            // locals_count covers: caller args + cells + c_return + out values
            let out_count = out_param_indices.len() as u16;
            func.locals_count = non_out_count as u16 + out_count + 1 + out_count;
        } else {
            func.locals_count = total_c_arg_count;
        }
        let (ref_params, ref_mutates) = Self::native_param_reference_contract(def);
        if has_out_params {
            // Filter ref_params/ref_mutates to only include non-out params
            let mut filtered_ref_params = Vec::new();
            let mut filtered_ref_mutates = Vec::new();
            for (i, (rp, rm)) in ref_params.iter().zip(ref_mutates.iter()).enumerate() {
                if !out_param_indices.contains(&i) {
                    filtered_ref_params.push(*rp);
                    filtered_ref_mutates.push(*rm);
                }
            }
            func.ref_params = filtered_ref_params;
            func.ref_mutates = filtered_ref_mutates;
        } else {
            func.ref_params = ref_params;
            func.ref_mutates = ref_mutates;
        }
        // Update param_names to only include non-out params for caller-visible signature
        if has_out_params {
            let visible_names: Vec<String> = def
                .params
                .iter()
                .enumerate()
                .filter(|(i, _)| !out_param_indices.contains(i))
                .flat_map(|(_, p)| p.get_identifiers())
                .collect();
            func.param_names = visible_names;
        }

        // Finalize and register the extern stub blob.
        self.finalize_current_blob(func_idx);
        self.current_blob_builder = saved_blob_builder;

        // Patch the jump-over to land here (after the function body)
        self.patch_jump(jump_over);

        // Store the function binding so the name resolves at call sites
        let binding_idx = self.get_or_create_module_binding(&def.name);
        let func_const = self
            .program
            .add_constant(Constant::Function(func_idx as u16));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(func_const)),
        ));
        self.emit(Instruction::new(
            OpCode::StoreModuleBinding,
            Some(Operand::ModuleBinding(binding_idx)),
        ));

        // Check for annotation-based wrapping on foreign functions (e.g. @remote).
        // This mirrors the annotation wrapping in compile_function for regular fns.
        let foreign_annotations: Vec<_> = def
            .annotations
            .iter()
            .filter_map(|ann| {
                self.program
                    .compiled_annotations
                    .get(&ann.name)
                    .filter(|c| c.before_handler.is_some() || c.after_handler.is_some())
                    .cloned()
            })
            .collect();

        if let Some(compiled_ann) = foreign_annotations.into_iter().next() {
            let ann_arg_exprs: Vec<_> = def
                .annotations
                .iter()
                .find(|a| a.name == compiled_ann.name)
                .map(|a| a.args.clone())
                .unwrap_or_default();

            // The foreign stub at func_idx is the impl
            let impl_idx = func_idx as u16;

            // Create a new function slot for the annotation wrapper
            let wrapper_func_idx = self.program.functions.len();
            let wrapper_param_names: Vec<String> = def
                .params
                .iter()
                .enumerate()
                .filter(|(i, _)| !out_param_indices.contains(i))
                .flat_map(|(_, p)| p.get_identifiers())
                .collect();
            self.program.functions.push(crate::bytecode::Function {
                name: format!("{}___ann_wrapper", def.name),
                arity: caller_visible_arity,
                param_names: wrapper_param_names,
                locals_count: 0,
                entry_point: 0,
                body_length: 0,
                is_closure: false,
                captures_count: 0,
                is_async: def.is_async,
                ref_params: Vec::new(),
                ref_mutates: Vec::new(),
                mutable_captures: Vec::new(),
                frame_descriptor: None,
                osr_entry_points: Vec::new(),
            });

            // Build a synthetic FunctionDef for the annotation wrapper machinery.
            // Only params visible to the caller (non-out) are included.
            let wrapper_params: Vec<_> = def
                .params
                .iter()
                .enumerate()
                .filter(|(i, _)| !out_param_indices.contains(i))
                .map(|(_, p)| p.clone())
                .collect();
            let synthetic_def = FunctionDef {
                name: def.name.clone(),
                name_span: def.name_span,
                declaring_module_path: None,
                doc_comment: None,
                params: wrapper_params,
                return_type: def.return_type.clone(),
                body: vec![],
                type_params: def.type_params.clone(),
                annotations: def.annotations.clone(),
                where_clause: None,
                is_async: def.is_async,
                is_comptime: false,
            };

            self.compile_annotation_wrapper(
                &synthetic_def,
                wrapper_func_idx,
                impl_idx,
                &compiled_ann,
                &ann_arg_exprs,
            )?;

            // Update module binding to point to the wrapper
            let wrapper_const = self
                .program
                .add_constant(Constant::Function(wrapper_func_idx as u16));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(wrapper_const)),
            ));
            self.emit(Instruction::new(
                OpCode::StoreModuleBinding,
                Some(Operand::ModuleBinding(binding_idx)),
            ));
        }

        Ok(())
    }

    /// Validate `out` parameter constraints on a foreign function definition.
    fn validate_out_params(&self, def: &shape_ast::ast::ForeignFunctionDef) -> Result<()> {
        for param in &def.params {
            if !param.is_out {
                continue;
            }
            let param_name = param.simple_name().unwrap_or("_");

            // out params only valid on extern C functions
            if !def.is_native_abi() {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Function '{}': `out` parameter '{}' is only valid on `extern C` declarations",
                        def.name, param_name
                    ),
                    location: Some(self.span_to_source_location(param.span())),
                });
            }

            // Must have type ptr
            let is_ptr = param
                .type_annotation
                .as_ref()
                .map(|ann| matches!(ann, shape_ast::ast::TypeAnnotation::Basic(n) if n == "ptr"))
                .unwrap_or(false);
            if !is_ptr {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Function '{}': `out` parameter '{}' must have type `ptr`",
                        def.name, param_name
                    ),
                    location: Some(self.span_to_source_location(param.span())),
                });
            }

            // Cannot combine with const or &
            if param.is_const {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Function '{}': `out` parameter '{}' cannot be `const`",
                        def.name, param_name
                    ),
                    location: Some(self.span_to_source_location(param.span())),
                });
            }
            if param.is_reference {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Function '{}': `out` parameter '{}' cannot be a reference (`&`)",
                        def.name, param_name
                    ),
                    location: Some(self.span_to_source_location(param.span())),
                });
            }

            // Cannot have default value
            if param.default_value.is_some() {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Function '{}': `out` parameter '{}' cannot have a default value",
                        def.name, param_name
                    ),
                    location: Some(self.span_to_source_location(param.span())),
                });
            }
        }
        Ok(())
    }

    /// Emit the out-param stub: allocate cells, call C, read back, free cells, build tuple.
    ///
    /// Local layout:
    ///   [0..N)           = caller-visible (non-out) params
    ///   [N..N+M)         = cells for out params
    ///   [N+M]            = C return value
    ///   [N+M+1..N+2M+1) = out param read-back values
    fn emit_out_param_stub(
        &mut self,
        def: &shape_ast::ast::ForeignFunctionDef,
        _func_idx: usize,
        foreign_idx: u16,
        out_param_indices: &[usize],
    ) -> Result<()> {
        use crate::bytecode::BuiltinFunction;

        let out_count = out_param_indices.len() as u16;
        let non_out_count = (def.params.len() - out_count as usize) as u16;
        let total_c_args = def.params.len() as u16;

        // Locals: [caller_args(0..N), cells(N..N+M), c_ret(N+M), out_vals(N+M+1..N+2M+1)]
        let cell_base = non_out_count;
        let c_ret_local = non_out_count + out_count;
        let out_val_base = c_ret_local + 1;

        // Helper to emit a builtin call with arg count
        macro_rules! emit_builtin {
            ($builtin:expr, $argc:expr) => {{
                let argc_const = self.program.add_constant(Constant::Number($argc as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(argc_const)),
                ));
                self.emit(Instruction::new(
                    OpCode::BuiltinCall,
                    Some(Operand::Builtin($builtin)),
                ));
            }};
        }

        // 1. Allocate and initialize cells for each out param
        for i in 0..out_count {
            // ptr_new_cell() -> cell
            emit_builtin!(BuiltinFunction::NativePtrNewCell, 0);
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(cell_base + i)),
            ));

            // ptr_write(cell, 0) — initialize to 0
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(cell_base + i)),
            ));
            let zero_const = self.program.add_constant(Constant::Number(0.0));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(zero_const)),
            ));
            emit_builtin!(BuiltinFunction::NativePtrWritePtr, 2);
        }

        // 2. Push C call args in the original parameter order.
        //    Non-out params come from caller locals, out params use cell addresses.
        let mut out_idx = 0u16;
        for (i, param) in def.params.iter().enumerate() {
            if param.is_out {
                // Load the cell address for this out param
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(cell_base + out_idx)),
                ));
                out_idx += 1;
            } else {
                // Load the caller-visible arg. We need to compute the caller-local index.
                let caller_local = def.params[..i].iter().filter(|p| !p.is_out).count() as u16;
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(caller_local)),
                ));
            }
        }

        // 3. Call foreign function with total C arg count
        let c_arg_count_const = self
            .program
            .add_constant(Constant::Number(total_c_args as f64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(c_arg_count_const)),
        ));
        self.emit(Instruction::new(
            OpCode::CallForeign,
            Some(Operand::ForeignFunction(foreign_idx)),
        ));

        // Store C return value
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(c_ret_local)),
        ));

        // 4. Read back out param values from cells
        for i in 0..out_count {
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(cell_base + i)),
            ));
            emit_builtin!(BuiltinFunction::NativePtrReadPtr, 1);
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(out_val_base + i)),
            ));
        }

        // 5. Free cells
        for i in 0..out_count {
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(cell_base + i)),
            ));
            emit_builtin!(BuiltinFunction::NativePtrFreeCell, 1);
        }

        // 6. Build return value
        let is_void_return = def.return_type.as_ref().map_or(
            false,
            |ann| matches!(ann, shape_ast::ast::TypeAnnotation::Basic(n) if n == "void"),
        );

        if out_count == 1 && is_void_return {
            // Single out param + void return → return the out value directly
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(out_val_base)),
            ));
        } else {
            // Build tuple: (return_val, out_val1, out_val2, ...)
            // Push return value first (unless void)
            let mut tuple_size = out_count;
            if !is_void_return {
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(c_ret_local)),
                ));
                tuple_size += 1;
            }
            // Push out values
            for i in 0..out_count {
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(out_val_base + i)),
                ));
            }
            // Create array (used as tuple)
            self.emit(Instruction::new(
                OpCode::NewArray,
                Some(Operand::Count(tuple_size)),
            ));
        }

        self.emit(Instruction::simple(OpCode::ReturnValue));
        Ok(())
    }

    /// Walk a TypeAnnotation tree to find the first Object node.
    /// Unwraps `Result<T>`, `Generic{..}`, and `Vec<T>` wrappers.
    fn find_object_in_annotation(
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<&[shape_ast::ast::ObjectTypeField]> {
        use shape_ast::ast::TypeAnnotation;
        match ann {
            TypeAnnotation::Object(fields) => Some(fields),
            TypeAnnotation::Generic { args, .. } => {
                // Unwrap Result<T>, Option<T>, etc. — check inner type args
                args.iter().find_map(Self::find_object_in_annotation)
            }
            TypeAnnotation::Array(inner) => Self::find_object_in_annotation(inner),
            _ => None,
        }
    }

    /// Walk a TypeAnnotation tree to find the first Reference name.
    /// Unwraps `Result<T>`, `Generic{..}`, and `Array<T>` wrappers.
    fn find_reference_in_annotation(ann: &shape_ast::ast::TypeAnnotation) -> Option<&str> {
        use shape_ast::ast::TypeAnnotation;
        match ann {
            TypeAnnotation::Reference(name) => Some(name.as_str()),
            TypeAnnotation::Generic { args, .. } => {
                args.iter().find_map(Self::find_reference_in_annotation)
            }
            TypeAnnotation::Array(inner) => Self::find_reference_in_annotation(inner),
            _ => None,
        }
    }

    pub(super) fn native_ctype_from_annotation(
        ann: &shape_ast::ast::TypeAnnotation,
        is_return: bool,
    ) -> Option<String> {
        use shape_ast::ast::TypeAnnotation;
        match ann {
            TypeAnnotation::Array(inner) => {
                let elem = Self::native_slice_elem_ctype_from_annotation(inner)?;
                Some(format!("cslice<{elem}>"))
            }
            TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => match name.as_str() {
                "number" | "Number" | "float" | "f64" => Some("f64".to_string()),
                "f32" => Some("f32".to_string()),
                "int" | "integer" | "Int" | "Integer" | "i64" => Some("i64".to_string()),
                "i32" => Some("i32".to_string()),
                "i16" => Some("i16".to_string()),
                "i8" => Some("i8".to_string()),
                "u64" => Some("u64".to_string()),
                "u32" => Some("u32".to_string()),
                "u16" => Some("u16".to_string()),
                "u8" | "byte" => Some("u8".to_string()),
                "isize" => Some("isize".to_string()),
                "usize" => Some("usize".to_string()),
                "char" => Some("i8".to_string()),
                "bool" | "boolean" => Some("bool".to_string()),
                "string" | "str" => Some("cstring".to_string()),
                "cstring" => Some("cstring".to_string()),
                "ptr" | "pointer" => Some("ptr".to_string()),
                "void" if is_return => Some("void".to_string()),
                _ => None,
            },
            TypeAnnotation::Void if is_return => Some("void".to_string()),
            TypeAnnotation::Generic { name, args }
                if (name == "Vec" || name == "CSlice" || name == "CMutSlice")
                    && args.len() == 1 =>
            {
                let elem = Self::native_slice_elem_ctype_from_annotation(&args[0])?;
                if name == "CMutSlice" {
                    Some(format!("cmut_slice<{elem}>"))
                } else {
                    Some(format!("cslice<{elem}>"))
                }
            }
            TypeAnnotation::Generic { name, args } if name == "Option" && args.len() == 1 => {
                let inner = Self::native_ctype_from_annotation(&args[0], is_return)?;
                if inner == "cstring" {
                    Some("cstring?".to_string())
                } else {
                    None
                }
            }
            TypeAnnotation::Generic { name, args }
                if (name == "CView" || name == "CMut") && args.len() == 1 =>
            {
                let inner = match &args[0] {
                    TypeAnnotation::Reference(type_name) | TypeAnnotation::Basic(type_name) => {
                        type_name.clone()
                    }
                    _ => return None,
                };
                if name == "CView" {
                    Some(format!("cview<{inner}>"))
                } else {
                    Some(format!("cmut<{inner}>"))
                }
            }
            TypeAnnotation::Function { params, returns } if !is_return => {
                let mut callback_params = Vec::with_capacity(params.len());
                for param in params {
                    callback_params.push(Self::native_ctype_from_annotation(
                        &param.type_annotation,
                        false,
                    )?);
                }
                let callback_ret = Self::native_ctype_from_annotation(returns, true)?;
                Some(format!(
                    "callback(fn({}) -> {})",
                    callback_params.join(", "),
                    callback_ret
                ))
            }
            _ => None,
        }
    }

    pub(super) fn native_param_reference_contract(
        def: &shape_ast::ast::ForeignFunctionDef,
    ) -> (Vec<bool>, Vec<bool>) {
        let mut ref_params = vec![false; def.params.len()];
        let mut ref_mutates = vec![false; def.params.len()];
        if !def.is_native_abi() {
            return (ref_params, ref_mutates);
        }

        for (idx, param) in def.params.iter().enumerate() {
            let Some(annotation) = param.type_annotation.as_ref() else {
                continue;
            };
            if let Some(ctype) = Self::native_ctype_from_annotation(annotation, false)
                && Self::native_ctype_requires_mutable_reference(&ctype)
            {
                ref_params[idx] = true;
                ref_mutates[idx] = true;
            }
        }

        (ref_params, ref_mutates)
    }

    fn native_ctype_requires_mutable_reference(ctype: &str) -> bool {
        ctype.starts_with("cmut_slice<")
    }

    fn native_slice_elem_ctype_from_annotation(
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<String> {
        let elem = Self::native_ctype_from_annotation(ann, false)?;
        if Self::is_supported_native_slice_elem(&elem) {
            Some(elem)
        } else {
            None
        }
    }

    fn is_supported_native_slice_elem(ctype: &str) -> bool {
        matches!(
            ctype,
            "i8" | "u8"
                | "i16"
                | "u16"
                | "i32"
                | "i64"
                | "u32"
                | "u64"
                | "isize"
                | "usize"
                | "f32"
                | "f64"
                | "bool"
                | "ptr"
                | "cstring"
                | "cstring?"
        )
    }

    fn build_native_c_signature(&self, def: &shape_ast::ast::ForeignFunctionDef) -> Result<String> {
        let mut param_types = Vec::with_capacity(def.params.len());
        for (idx, param) in def.params.iter().enumerate() {
            let ann = param
                .type_annotation
                .as_ref()
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "extern native function '{}': parameter #{} must have a type annotation",
                        def.name, idx
                    ),
                    location: Some(self.span_to_source_location(param.span())),
                })?;
            let ctype = Self::native_ctype_from_annotation(ann, false).ok_or_else(|| {
                ShapeError::SemanticError {
                    message: format!(
                        "extern native function '{}': unsupported parameter type '{}' for C ABI",
                        def.name,
                        cabi_type_display(ann)
                    ),
                    location: Some(self.span_to_source_location(param.span())),
                }
            })?;
            param_types.push(ctype.to_string());
        }

        let ret_ann = def
            .return_type
            .as_ref()
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!(
                    "extern native function '{}': explicit return type is required",
                    def.name
                ),
                location: Some(self.span_to_source_location(def.name_span)),
            })?;
        let ret_type = Self::native_ctype_from_annotation(ret_ann, true).ok_or_else(|| {
            ShapeError::SemanticError {
                message: format!(
                    "extern native function '{}': unsupported return type '{}' for C ABI",
                    def.name,
                    cabi_type_display(ret_ann)
                ),
                location: Some(self.span_to_source_location(def.name_span)),
            }
        })?;

        Ok(format!("fn({}) -> {}", param_types.join(", "), ret_type))
    }

    fn resolve_native_library_alias(
        &self,
        requested: &str,
        declaring_package_key: Option<&str>,
    ) -> Result<String> {
        // Well-known aliases for standard system libraries.
        match requested {
            "c" | "libc" => {
                #[cfg(target_os = "linux")]
                return Ok("libc.so.6".to_string());
                #[cfg(target_os = "macos")]
                return Ok("libSystem.B.dylib".to_string());
                #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                return Ok("msvcrt.dll".to_string());
            }
            _ => {}
        }

        // Resolve package-local aliases through the shared native resolution context.
        if let Some(package_key) = declaring_package_key
            && let Some(resolutions) = &self.native_resolution_context
            && let Some(resolved) = resolutions
                .by_package_alias
                .get(&(package_key.to_string(), requested.to_string()))
        {
            return Ok(resolved.load_target.clone());
        }

        // Fall back to root-project native dependency declarations when compiling
        // a program that was not annotated with explicit package provenance.
        if declaring_package_key.is_none()
            && let Some(ref source_dir) = self.source_dir
            && let Some(project) = shape_runtime::project::find_project_root(source_dir)
            && let Ok(native_deps) = project.config.native_dependencies()
            && let Some(spec) = native_deps.get(requested)
            && let Some(resolved) = spec.resolve_for_host()
        {
            return Ok(resolved);
        }
        Ok(requested.to_string())
    }

    fn emit_annotation_lifecycle_calls(&mut self, func_def: &FunctionDef) -> Result<()> {
        if self.current_function.is_some() {
            return Ok(());
        }
        if func_def.annotations.is_empty() {
            return Ok(());
        }

        let self_fn_idx =
            self.find_function(&func_def.name)
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: format!(
                        "Internal error: function '{}' not found for annotation lifecycle dispatch",
                        func_def.name
                    ),
                    location: None,
                })? as u16;

        self.emit_annotation_lifecycle_calls_for_target(
            &func_def.annotations,
            &func_def.name,
            shape_ast::ast::functions::AnnotationTargetKind::Function,
            Some(self_fn_idx),
        )
    }

    pub(super) fn emit_annotation_lifecycle_calls_for_type(
        &mut self,
        type_name: &str,
        annotations: &[shape_ast::ast::Annotation],
    ) -> Result<()> {
        if self.current_function.is_some() || annotations.is_empty() {
            return Ok(());
        }
        self.emit_annotation_lifecycle_calls_for_target(
            annotations,
            type_name,
            shape_ast::ast::functions::AnnotationTargetKind::Type,
            Some(0),
        )
    }

    pub(super) fn emit_annotation_lifecycle_calls_for_module(
        &mut self,
        module_name: &str,
        annotations: &[shape_ast::ast::Annotation],
        target_id: Option<u16>,
    ) -> Result<()> {
        if self.current_function.is_some() || annotations.is_empty() {
            return Ok(());
        }
        self.emit_annotation_lifecycle_calls_for_target(
            annotations,
            module_name,
            shape_ast::ast::functions::AnnotationTargetKind::Module,
            target_id,
        )
    }

    fn emit_annotation_lifecycle_calls_for_target(
        &mut self,
        annotations: &[shape_ast::ast::Annotation],
        target_name: &str,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
        target_id: Option<u16>,
    ) -> Result<()> {
        for ann in annotations {
            let Some(compiled) = self.program.compiled_annotations.get(&ann.name).cloned() else {
                continue;
            };

            if let Some(on_define_id) = compiled.on_define_handler {
                self.emit_annotation_handler_call(
                    on_define_id,
                    ann,
                    target_name,
                    target_kind,
                    target_id,
                )?;
            }
            if let Some(metadata_id) = compiled.metadata_handler {
                self.emit_annotation_handler_call(
                    metadata_id,
                    ann,
                    target_name,
                    target_kind,
                    target_id,
                )?;
            }
        }

        Ok(())
    }

    fn emit_annotation_handler_call(
        &mut self,
        handler_id: u16,
        annotation: &shape_ast::ast::Annotation,
        target_name: &str,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
        target_id: Option<u16>,
    ) -> Result<()> {
        let handler = self
            .program
            .functions
            .get(handler_id as usize)
            .cloned()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Internal error: annotation handler function {} not found",
                    handler_id
                ),
                location: None,
            })?;
        let expected_base = 1 + annotation.args.len();
        let arity = handler.arity as usize;
        if arity < expected_base {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Internal error: annotation handler '{}' arity {} is smaller than required base args {}",
                    handler.name, arity, expected_base
                ),
                location: None,
            });
        }

        match target_kind {
            shape_ast::ast::functions::AnnotationTargetKind::Function => {
                let id = target_id.ok_or_else(|| ShapeError::RuntimeError {
                    message: "Internal error: missing function id for annotation handler call"
                        .to_string(),
                    location: None,
                })?;
                let self_ref = self.program.add_constant(Constant::Number(id as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(self_ref)),
                ));
            }
            _ => {
                self.emit_annotation_target_descriptor(target_name, target_kind, target_id)?;
            }
        }

        for ann_arg in &annotation.args {
            self.compile_expr(ann_arg)?;
        }

        for param_idx in expected_base..arity {
            let param_name = handler
                .param_names
                .get(param_idx)
                .map(|s| s.as_str())
                .unwrap_or_default();
            match param_name {
                "fn" | "target" => {
                    self.emit_annotation_target_descriptor(target_name, target_kind, target_id)?
                }
                "ctx" => self.emit_annotation_runtime_ctx()?,
                _ => {
                    self.emit(Instruction::simple(OpCode::PushNull));
                }
            }
        }

        let ac = self.program.add_constant(Constant::Number(arity as f64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(ac)),
        ));
        self.emit(Instruction::new(
            OpCode::Call,
            Some(Operand::Function(shape_value::FunctionId(handler_id))),
        ));
        self.record_blob_call(handler_id);
        self.emit(Instruction::simple(OpCode::Pop));
        Ok(())
    }

    fn annotation_target_kind_label(
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
    ) -> &'static str {
        match target_kind {
            shape_ast::ast::functions::AnnotationTargetKind::Function => "function",
            shape_ast::ast::functions::AnnotationTargetKind::Type => "type",
            shape_ast::ast::functions::AnnotationTargetKind::Module => "module",
            shape_ast::ast::functions::AnnotationTargetKind::Expression => "expression",
            shape_ast::ast::functions::AnnotationTargetKind::Block => "block",
            shape_ast::ast::functions::AnnotationTargetKind::AwaitExpr => "await_expr",
            shape_ast::ast::functions::AnnotationTargetKind::Binding => "binding",
        }
    }

    fn emit_annotation_runtime_ctx(&mut self) -> Result<()> {
        let empty_schema_id = self.type_tracker.register_inline_object_schema(&[]);
        if empty_schema_id > u16::MAX as u32 {
            return Err(ShapeError::RuntimeError {
                message: "Internal error: annotation ctx schema id overflow".to_string(),
                location: None,
            });
        }
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: empty_schema_id as u16,
                field_count: 0,
            }),
        ));
        self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));

        let ctx_schema_id = self.type_tracker.register_inline_object_schema_typed(&[
            ("state", FieldType::Any),
            ("event_log", FieldType::Array(Box::new(FieldType::Any))),
        ]);
        if ctx_schema_id > u16::MAX as u32 {
            return Err(ShapeError::RuntimeError {
                message: "Internal error: annotation ctx schema id overflow".to_string(),
                location: None,
            });
        }
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: ctx_schema_id as u16,
                field_count: 2,
            }),
        ));
        Ok(())
    }

    fn emit_annotation_target_descriptor(
        &mut self,
        target_name: &str,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
        target_id: Option<u16>,
    ) -> Result<()> {
        let name_const = self
            .program
            .add_constant(Constant::String(target_name.to_string()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(name_const)),
        ));
        let kind_const = self.program.add_constant(Constant::String(
            Self::annotation_target_kind_label(target_kind).to_string(),
        ));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(kind_const)),
        ));
        if let Some(id) = target_id {
            let id_const = self.program.add_constant(Constant::Number(id as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(id_const)),
            ));
        } else {
            self.emit(Instruction::simple(OpCode::PushNull));
        }

        let fn_schema_id = self.type_tracker.register_inline_object_schema_typed(&[
            ("name", FieldType::String),
            ("kind", FieldType::String),
            ("id", FieldType::I64),
        ]);
        if fn_schema_id > u16::MAX as u32 {
            return Err(ShapeError::RuntimeError {
                message: "Internal error: annotation fn schema id overflow".to_string(),
                location: None,
            });
        }
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: fn_schema_id as u16,
                field_count: 3,
            }),
        ));
        Ok(())
    }

    /// Execute comptime annotation handlers for a function definition.
    ///
    /// When an annotation has a `comptime pre/post(...) { ... }` handler, self builds
    /// a ComptimeTarget from the function definition and executes the handler body
    /// at compile time with the target object bound to the handler parameter.
    fn execute_comptime_handlers(&mut self, func_def: &mut FunctionDef) -> Result<bool> {
        let mut removed = false;
        let annotations = func_def.annotations.clone();

        // Phase 1: comptime pre
        for ann in &annotations {
            let compiled = self.program.compiled_annotations.get(&ann.name).cloned();
            if let Some(compiled) = compiled {
                if let Some(handler) = compiled.comptime_pre_handler {
                    if self.execute_function_comptime_handler(
                        ann,
                        &handler,
                        &compiled.param_names,
                        func_def,
                    )? {
                        removed = true;
                        break;
                    }
                }
            }
        }

        // Phase 2: comptime post
        if !removed {
            for ann in &annotations {
                let compiled = self.program.compiled_annotations.get(&ann.name).cloned();
                if let Some(compiled) = compiled {
                    if let Some(handler) = compiled.comptime_post_handler {
                        if self.execute_function_comptime_handler(
                            ann,
                            &handler,
                            &compiled.param_names,
                            func_def,
                        )? {
                            removed = true;
                            break;
                        }
                    }
                }
            }
        }

        Ok(removed)
    }

    fn execute_function_comptime_handler(
        &mut self,
        annotation: &shape_ast::ast::Annotation,
        handler: &shape_ast::ast::AnnotationHandler,
        annotation_def_param_names: &[String],
        func_def: &mut FunctionDef,
    ) -> Result<bool> {
        // Build the target object from the function definition
        let target = super::comptime_target::ComptimeTarget::from_function(func_def);
        let target_value = target.to_nanboxed();
        let target_name = func_def.name.clone();
        let handler_span = handler.span;
        let const_bindings = self
            .specialization_const_bindings
            .get(&target_name)
            .cloned()
            .unwrap_or_default();

        let execution = self.execute_comptime_annotation_handler(
            annotation,
            handler,
            target_value,
            annotation_def_param_names,
            &const_bindings,
        )?;

        self.process_comptime_directives_for_function(execution.directives, &target_name, func_def)
            .map_err(|e| ShapeError::RuntimeError {
                message: format!(
                    "Comptime handler '{}' directive processing failed: {}",
                    annotation.name, e
                ),
                location: Some(self.span_to_source_location(handler_span)),
            })
    }

    pub(super) fn execute_comptime_annotation_handler(
        &mut self,
        annotation: &shape_ast::ast::Annotation,
        handler: &shape_ast::ast::AnnotationHandler,
        target_value: ValueWord,
        annotation_def_param_names: &[String],
        const_bindings: &[(String, shape_value::ValueWord)],
    ) -> Result<super::comptime::ComptimeExecutionResult> {
        let handler_span = handler.span;
        let extensions: Vec<_> = self
            .extension_registry
            .as_ref()
            .map(|r| r.as_ref().clone())
            .unwrap_or_default();
        let trait_impls = self.type_inference.env.trait_impl_keys();
        let known_type_symbols: std::collections::HashSet<String> = self
            .struct_types
            .keys()
            .chain(self.type_aliases.keys())
            .cloned()
            .collect();
        let mut comptime_helpers = self.collect_comptime_helpers();
        comptime_helpers.extend(self.collect_scoped_helpers_for_expr(&handler.body));
        comptime_helpers.sort_by(|a, b| a.name.cmp(&b.name));
        comptime_helpers.dedup_by(|a, b| a.name == b.name);

        super::comptime::execute_comptime_with_annotation_handler(
            &handler.body,
            &handler.params,
            target_value,
            &annotation.args,
            annotation_def_param_names,
            const_bindings,
            &comptime_helpers,
            &extensions,
            trait_impls,
            known_type_symbols,
        )
        .map_err(|e| ShapeError::RuntimeError {
            message: format!(
                "Comptime handler '{}' failed: {}",
                annotation.name,
                super::helpers::strip_error_prefix(&e)
            ),
            location: Some(self.span_to_source_location(handler_span)),
        })
    }

    fn collect_scoped_helpers_for_expr(&self, expr: &Expr) -> Vec<FunctionDef> {
        let mut pending_names = Vec::new();
        let mut seed_names = HashSet::new();
        Self::collect_scoped_names_in_expr(expr, &mut seed_names);
        pending_names.extend(seed_names.into_iter());

        let mut visited = HashSet::new();
        let mut helpers = Vec::new();

        while let Some(name) = pending_names.pop() {
            if !visited.insert(name.clone()) {
                continue;
            }
            let Some(def) = self.function_defs.get(&name) else {
                continue;
            };
            helpers.push(def.clone());
            for stmt in &def.body {
                let mut nested = HashSet::new();
                Self::collect_scoped_names_in_statement(stmt, &mut nested);
                pending_names.extend(nested.into_iter().filter(|n| !visited.contains(n)));
            }
        }

        helpers
    }

    fn collect_scoped_names_in_statement(stmt: &Statement, names: &mut HashSet<String>) {
        match stmt {
            Statement::Return(Some(expr), _) => Self::collect_scoped_names_in_expr(expr, names),
            Statement::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Statement::Assignment(assign, _) => {
                Self::collect_scoped_names_in_expr(&assign.value, names)
            }
            Statement::Expression(expr, _) => Self::collect_scoped_names_in_expr(expr, names),
            Statement::For(loop_expr, _) => {
                match &loop_expr.init {
                    shape_ast::ast::ForInit::ForIn { iter, .. } => {
                        Self::collect_scoped_names_in_expr(iter, names);
                    }
                    shape_ast::ast::ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        Self::collect_scoped_names_in_statement(init, names);
                        Self::collect_scoped_names_in_expr(condition, names);
                        Self::collect_scoped_names_in_expr(update, names);
                    }
                }
                for body_stmt in &loop_expr.body {
                    Self::collect_scoped_names_in_statement(body_stmt, names);
                }
            }
            Statement::While(loop_expr, _) => {
                Self::collect_scoped_names_in_expr(&loop_expr.condition, names);
                for body_stmt in &loop_expr.body {
                    Self::collect_scoped_names_in_statement(body_stmt, names);
                }
            }
            Statement::If(if_stmt, _) => {
                Self::collect_scoped_names_in_expr(&if_stmt.condition, names);
                for body_stmt in &if_stmt.then_body {
                    Self::collect_scoped_names_in_statement(body_stmt, names);
                }
                if let Some(else_body) = &if_stmt.else_body {
                    for body_stmt in else_body {
                        Self::collect_scoped_names_in_statement(body_stmt, names);
                    }
                }
            }
            Statement::SetReturnExpr { expression, .. }
            | Statement::SetParamValue { expression, .. }
            | Statement::ReplaceBodyExpr { expression, .. }
            | Statement::ReplaceModuleExpr { expression, .. } => {
                Self::collect_scoped_names_in_expr(expression, names);
            }
            Statement::ReplaceBody { body, .. } => {
                for stmt in body {
                    Self::collect_scoped_names_in_statement(stmt, names);
                }
            }
            _ => {}
        }
    }

    fn collect_scoped_names_in_expr(expr: &Expr, names: &mut HashSet<String>) {
        match expr {
            Expr::MethodCall {
                receiver,
                method,
                args,
                named_args,
                ..
            } => {
                if let Expr::Identifier(namespace, _) = receiver.as_ref() {
                    names.insert(format!("{}::{}", namespace, method));
                }
                Self::collect_scoped_names_in_expr(receiver, names);
                for arg in args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                for (_, value) in named_args {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::FunctionCall {
                name,
                args,
                named_args,
                ..
            } => {
                if name.contains("::") {
                    names.insert(name.clone());
                }
                for arg in args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                for (_, value) in named_args {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                Self::collect_scoped_names_in_expr(left, names);
                Self::collect_scoped_names_in_expr(right, names);
            }
            Expr::UnaryOp { operand, .. }
            | Expr::Spread(operand, _)
            | Expr::TryOperator(operand, _)
            | Expr::Await(operand, _)
            | Expr::Reference { expr: operand, .. }
            | Expr::AsyncScope(operand, _)
            | Expr::DataRelativeAccess {
                reference: operand, ..
            } => {
                Self::collect_scoped_names_in_expr(operand, names);
            }
            Expr::PropertyAccess { object, .. } => {
                Self::collect_scoped_names_in_expr(object, names)
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                Self::collect_scoped_names_in_expr(object, names);
                Self::collect_scoped_names_in_expr(index, names);
                if let Some(end) = end_index {
                    Self::collect_scoped_names_in_expr(end, names);
                }
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                Self::collect_scoped_names_in_expr(condition, names);
                Self::collect_scoped_names_in_expr(then_expr, names);
                if let Some(else_expr) = else_expr {
                    Self::collect_scoped_names_in_expr(else_expr, names);
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        ObjectEntry::Field { value, .. } | ObjectEntry::Spread(value) => {
                            Self::collect_scoped_names_in_expr(value, names);
                        }
                    }
                }
            }
            Expr::Array(values, _) => {
                for value in values {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::ListComprehension(comp, _) => {
                Self::collect_scoped_names_in_expr(&comp.element, names);
                for clause in &comp.clauses {
                    Self::collect_scoped_names_in_expr(&clause.iterable, names);
                    if let Some(filter) = &clause.filter {
                        Self::collect_scoped_names_in_expr(filter, names);
                    }
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    match item {
                        shape_ast::ast::BlockItem::VariableDecl(decl) => {
                            if let Some(value) = &decl.value {
                                Self::collect_scoped_names_in_expr(value, names);
                            }
                        }
                        shape_ast::ast::BlockItem::Assignment(assign) => {
                            Self::collect_scoped_names_in_expr(&assign.value, names);
                        }
                        shape_ast::ast::BlockItem::Statement(stmt) => {
                            Self::collect_scoped_names_in_statement(stmt, names);
                        }
                        shape_ast::ast::BlockItem::Expression(expr) => {
                            Self::collect_scoped_names_in_expr(expr, names);
                        }
                    }
                }
            }
            Expr::TypeAssertion {
                expr,
                meta_param_overrides,
                ..
            } => {
                Self::collect_scoped_names_in_expr(expr, names);
                if let Some(overrides) = meta_param_overrides {
                    for value in overrides.values() {
                        Self::collect_scoped_names_in_expr(value, names);
                    }
                }
            }
            Expr::InstanceOf { expr, .. } => Self::collect_scoped_names_in_expr(expr, names),
            Expr::FunctionExpr { body, .. } => {
                for stmt in body {
                    Self::collect_scoped_names_in_statement(stmt, names);
                }
            }
            Expr::If(if_expr, _) => {
                Self::collect_scoped_names_in_expr(&if_expr.condition, names);
                Self::collect_scoped_names_in_expr(&if_expr.then_branch, names);
                if let Some(else_branch) = &if_expr.else_branch {
                    Self::collect_scoped_names_in_expr(else_branch, names);
                }
            }
            Expr::While(while_expr, _) => {
                Self::collect_scoped_names_in_expr(&while_expr.condition, names);
                Self::collect_scoped_names_in_expr(&while_expr.body, names);
            }
            Expr::For(for_expr, _) => {
                Self::collect_scoped_names_in_expr(&for_expr.iterable, names);
                Self::collect_scoped_names_in_expr(&for_expr.body, names);
            }
            Expr::Loop(loop_expr, _) => Self::collect_scoped_names_in_expr(&loop_expr.body, names),
            Expr::Let(let_expr, _) => {
                if let Some(value) = &let_expr.value {
                    Self::collect_scoped_names_in_expr(value, names);
                }
                Self::collect_scoped_names_in_expr(&let_expr.body, names);
            }
            Expr::Assign(assign_expr, _) => {
                Self::collect_scoped_names_in_expr(&assign_expr.target, names);
                Self::collect_scoped_names_in_expr(&assign_expr.value, names);
            }
            Expr::Break(Some(value), _) | Expr::Return(Some(value), _) => {
                Self::collect_scoped_names_in_expr(value, names);
            }
            Expr::Match(match_expr, _) => {
                Self::collect_scoped_names_in_expr(&match_expr.scrutinee, names);
                for arm in &match_expr.arms {
                    if let Some(guard) = &arm.guard {
                        Self::collect_scoped_names_in_expr(guard, names);
                    }
                    Self::collect_scoped_names_in_expr(&arm.body, names);
                }
            }
            Expr::Range { start, end, .. } => {
                if let Some(start) = start {
                    Self::collect_scoped_names_in_expr(start, names);
                }
                if let Some(end) = end {
                    Self::collect_scoped_names_in_expr(end, names);
                }
            }
            Expr::TimeframeContext { expr, .. } | Expr::UsingImpl { expr, .. } => {
                Self::collect_scoped_names_in_expr(expr, names);
            }
            Expr::SimulationCall { params, .. } => {
                for (_, value) in params {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::WindowExpr(window_expr, _) => {
                use shape_ast::ast::WindowFunction;

                match &window_expr.function {
                    WindowFunction::Lag { expr, default, .. }
                    | WindowFunction::Lead { expr, default, .. } => {
                        Self::collect_scoped_names_in_expr(expr, names);
                        if let Some(default) = default {
                            Self::collect_scoped_names_in_expr(default, names);
                        }
                    }
                    WindowFunction::FirstValue(expr)
                    | WindowFunction::LastValue(expr)
                    | WindowFunction::Sum(expr)
                    | WindowFunction::Avg(expr)
                    | WindowFunction::Min(expr)
                    | WindowFunction::Max(expr) => {
                        Self::collect_scoped_names_in_expr(expr, names);
                    }
                    WindowFunction::NthValue(expr, _) => {
                        Self::collect_scoped_names_in_expr(expr, names);
                    }
                    WindowFunction::Count(Some(expr)) => {
                        Self::collect_scoped_names_in_expr(expr, names);
                    }
                    WindowFunction::Count(None)
                    | WindowFunction::RowNumber
                    | WindowFunction::Rank
                    | WindowFunction::DenseRank
                    | WindowFunction::Ntile(_) => {}
                }

                for expr in &window_expr.over.partition_by {
                    Self::collect_scoped_names_in_expr(expr, names);
                }
                if let Some(order_by) = &window_expr.over.order_by {
                    for (expr, _) in &order_by.columns {
                        Self::collect_scoped_names_in_expr(expr, names);
                    }
                }
            }
            Expr::FromQuery(from_query, _) => {
                Self::collect_scoped_names_in_expr(&from_query.source, names);
                for clause in &from_query.clauses {
                    match clause {
                        shape_ast::ast::QueryClause::Where(expr) => {
                            Self::collect_scoped_names_in_expr(expr, names);
                        }
                        shape_ast::ast::QueryClause::OrderBy(specs) => {
                            for spec in specs {
                                Self::collect_scoped_names_in_expr(&spec.key, names);
                            }
                        }
                        shape_ast::ast::QueryClause::GroupBy { element, key, .. } => {
                            Self::collect_scoped_names_in_expr(element, names);
                            Self::collect_scoped_names_in_expr(key, names);
                        }
                        shape_ast::ast::QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            Self::collect_scoped_names_in_expr(source, names);
                            Self::collect_scoped_names_in_expr(left_key, names);
                            Self::collect_scoped_names_in_expr(right_key, names);
                        }
                        shape_ast::ast::QueryClause::Let { value, .. } => {
                            Self::collect_scoped_names_in_expr(value, names);
                        }
                    }
                }
                Self::collect_scoped_names_in_expr(&from_query.select, names);
            }
            Expr::StructLiteral { fields, .. } => {
                for (_, value) in fields {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::Join(join_expr, _) => {
                for branch in &join_expr.branches {
                    Self::collect_scoped_names_in_expr(&branch.expr, names);
                    for ann in &branch.annotations {
                        for arg in &ann.args {
                            Self::collect_scoped_names_in_expr(arg, names);
                        }
                    }
                }
            }
            Expr::Annotated {
                annotation, target, ..
            } => {
                for arg in &annotation.args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                Self::collect_scoped_names_in_expr(target, names);
            }
            Expr::AsyncLet(async_let, _) => {
                Self::collect_scoped_names_in_expr(&async_let.expr, names)
            }
            Expr::Comptime(stmts, _) => {
                for stmt in stmts {
                    Self::collect_scoped_names_in_statement(stmt, names);
                }
            }
            Expr::ComptimeFor(comptime_for, _) => {
                Self::collect_scoped_names_in_expr(&comptime_for.iterable, names);
                for stmt in &comptime_for.body {
                    Self::collect_scoped_names_in_statement(stmt, names);
                }
            }
            Expr::EnumConstructor { payload, .. } => match payload {
                shape_ast::ast::EnumConstructorPayload::Unit => {}
                shape_ast::ast::EnumConstructorPayload::Tuple(values) => {
                    for value in values {
                        Self::collect_scoped_names_in_expr(value, names);
                    }
                }
                shape_ast::ast::EnumConstructorPayload::Struct(fields) => {
                    for (_, value) in fields {
                        Self::collect_scoped_names_in_expr(value, names);
                    }
                }
            },
            Expr::TableRows(rows, _) => {
                for row in rows {
                    for elem in row {
                        Self::collect_scoped_names_in_expr(elem, names);
                    }
                }
            }
            Expr::Literal(..)
            | Expr::Identifier(..)
            | Expr::DataRef(..)
            | Expr::DataDateTimeRef(..)
            | Expr::TimeRef(..)
            | Expr::DateTime(..)
            | Expr::PatternRef(..)
            | Expr::Duration(..)
            | Expr::Break(None, _)
            | Expr::Return(None, _)
            | Expr::Continue(..)
            | Expr::Unit(..) => {}
        }
    }

    pub(super) fn apply_comptime_extend(
        &mut self,
        mut extend: shape_ast::ast::ExtendStatement,
        target_name: &str,
    ) -> Result<()> {
        match &mut extend.type_name {
            shape_ast::ast::TypeName::Simple(name) if name == "target" => {
                *name = target_name.to_string();
            }
            shape_ast::ast::TypeName::Generic { name, .. } if name == "target" => {
                *name = target_name.to_string();
            }
            _ => {}
        }

        for method in &extend.methods {
            let func_def = self.desugar_extend_method(method, &extend.type_name)?;
            self.register_function(&func_def)?;
            self.compile_function_body(&func_def)?;
        }
        Ok(())
    }

    pub(super) fn process_comptime_directives(
        &mut self,
        directives: Vec<super::comptime_builtins::ComptimeDirective>,
        target_name: &str,
    ) -> std::result::Result<bool, String> {
        let mut removed = false;
        for directive in directives {
            match directive {
                super::comptime_builtins::ComptimeDirective::Extend(extend) => {
                    self.apply_comptime_extend(extend, target_name)
                        .map_err(|e| e.to_string())?;
                }
                super::comptime_builtins::ComptimeDirective::RemoveTarget => {
                    removed = true;
                    break;
                }
                super::comptime_builtins::ComptimeDirective::SetParamType { .. }
                | super::comptime_builtins::ComptimeDirective::SetParamValue { .. } => {
                    return Err(
                        "`set param` directives are only valid when compiling function targets"
                            .to_string(),
                    );
                }
                super::comptime_builtins::ComptimeDirective::SetReturnType { .. } => {
                    return Err(
                        "`set return` directives are only valid when compiling function targets"
                            .to_string(),
                    );
                }
                super::comptime_builtins::ComptimeDirective::ReplaceBody { .. } => {
                    return Err(
                        "`replace body` directives are only valid when compiling function targets"
                            .to_string(),
                    );
                }
                super::comptime_builtins::ComptimeDirective::ReplaceModule { .. } => {
                    return Err(
                        "`replace module` directives are only valid when compiling module targets"
                            .to_string(),
                    );
                }
            }
        }
        Ok(removed)
    }

    fn process_comptime_directives_for_function(
        &mut self,
        directives: Vec<super::comptime_builtins::ComptimeDirective>,
        target_name: &str,
        func_def: &mut FunctionDef,
    ) -> std::result::Result<bool, String> {
        let mut removed = false;
        for directive in directives {
            match directive {
                super::comptime_builtins::ComptimeDirective::Extend(extend) => {
                    self.apply_comptime_extend(extend, target_name)
                        .map_err(|e| e.to_string())?;
                }
                super::comptime_builtins::ComptimeDirective::RemoveTarget => {
                    removed = true;
                    break;
                }
                super::comptime_builtins::ComptimeDirective::SetParamType {
                    param_name,
                    type_annotation,
                } => {
                    let maybe_param = func_def
                        .params
                        .iter_mut()
                        .find(|p| p.simple_name() == Some(param_name.as_str()));
                    let Some(param) = maybe_param else {
                        return Err(format!(
                            "comptime directive referenced unknown parameter '{}'",
                            param_name
                        ));
                    };
                    if let Some(existing) = &param.type_annotation {
                        if existing != &type_annotation {
                            return Err(format!(
                                "cannot override explicit type of parameter '{}'",
                                param_name
                            ));
                        }
                    } else {
                        param.type_annotation = Some(type_annotation);
                    }
                }
                super::comptime_builtins::ComptimeDirective::SetParamValue {
                    param_name,
                    value,
                } => {
                    let maybe_param = func_def
                        .params
                        .iter_mut()
                        .find(|p| p.simple_name() == Some(param_name.as_str()));
                    let Some(param) = maybe_param else {
                        return Err(format!(
                            "comptime directive referenced unknown parameter '{}'",
                            param_name
                        ));
                    };
                    // Convert the comptime ValueWord to an AST literal expression
                    let default_expr = if let Some(i) = value.as_i64() {
                        Expr::Literal(Literal::Int(i), Span::DUMMY)
                    } else if let Some(n) = value.as_number_coerce() {
                        Expr::Literal(Literal::Number(n), Span::DUMMY)
                    } else if let Some(b) = value.as_bool() {
                        Expr::Literal(Literal::Bool(b), Span::DUMMY)
                    } else if let Some(s) = value.as_str() {
                        Expr::Literal(Literal::String(s.to_string()), Span::DUMMY)
                    } else {
                        Expr::Literal(Literal::None, Span::DUMMY)
                    };
                    param.default_value = Some(default_expr);
                }
                super::comptime_builtins::ComptimeDirective::SetReturnType { type_annotation } => {
                    if let Some(existing) = &func_def.return_type {
                        if existing != &type_annotation {
                            return Err("cannot override explicit function return type annotation"
                                .to_string());
                        }
                    } else {
                        func_def.return_type = Some(type_annotation);
                    }
                }
                super::comptime_builtins::ComptimeDirective::ReplaceBody { body } => {
                    // Create a shadow function from the original body so the
                    // replacement can call __original__ to invoke the original
                    // implementation.
                    let shadow_name = format!("__original__{}", func_def.name);
                    let shadow_def = FunctionDef {
                        name: shadow_name.clone(),
                        name_span: func_def.name_span,
                        declaring_module_path: func_def.declaring_module_path.clone(),
                        doc_comment: None,
                        params: func_def.params.clone(),
                        return_type: func_def.return_type.clone(),
                        body: func_def.body.clone(),
                        type_params: func_def.type_params.clone(),
                        annotations: Vec::new(),
                        where_clause: None,
                        is_async: func_def.is_async,
                        is_comptime: func_def.is_comptime,
                    };
                    self.register_function(&shadow_def)
                        .map_err(|e| e.to_string())?;
                    self.compile_function_body(&shadow_def)
                        .map_err(|e| e.to_string())?;

                    // Register alias so __original__ resolves to the shadow function.
                    self.function_aliases
                        .insert("__original__".to_string(), shadow_name);

                    // Inject `let args = [param1, param2, ...]` at the start of the
                    // replacement body so the replacement can forward all arguments.
                    let param_idents: Vec<Expr> = func_def
                        .params
                        .iter()
                        .filter_map(|p| {
                            p.simple_name()
                                .map(|n| Expr::Identifier(n.to_string(), Span::DUMMY))
                        })
                        .collect();
                    let args_decl = Statement::VariableDecl(
                        VariableDecl {
                            kind: VarKind::Let,
                            is_mut: false,
                            pattern: DestructurePattern::Identifier(
                                "args".to_string(),
                                Span::DUMMY,
                            ),
                            type_annotation: None,
                            value: Some(Expr::Array(param_idents, Span::DUMMY)),
                            ownership: Default::default(),
                        },
                        Span::DUMMY,
                    );
                    let mut new_body = vec![args_decl];
                    new_body.extend(body);
                    func_def.body = new_body;
                }
                super::comptime_builtins::ComptimeDirective::ReplaceModule { .. } => {
                    return Err(
                        "`replace module` directives are only valid when compiling module targets"
                            .to_string(),
                    );
                }
            }
        }
        Ok(removed)
    }

    /// Validate that all annotations on a function are allowed for function targets.
    fn validate_annotation_targets(&self, func_def: &FunctionDef) -> Result<()> {
        for ann in &func_def.annotations {
            self.validate_annotation_target_usage(
                ann,
                shape_ast::ast::functions::AnnotationTargetKind::Function,
                func_def.name_span,
            )?;
        }
        Ok(())
    }

    /// Find ALL compiled annotations with before/after handlers on self function.
    /// Returns them in declaration order (first annotation = outermost wrapper).
    fn find_compiled_annotations(
        &self,
        func_def: &FunctionDef,
    ) -> Vec<crate::bytecode::CompiledAnnotation> {
        let mut result = Vec::new();
        for ann in &func_def.annotations {
            if let Some(compiled) = self.program.compiled_annotations.get(&ann.name) {
                if compiled.before_handler.is_some() || compiled.after_handler.is_some() {
                    result.push(compiled.clone());
                }
            }
        }
        result
    }

    /// Compile a function with multiple chained annotations.
    ///
    /// For `@a @b function foo(x) { body }`:
    /// 1. Compile original body as `foo___impl`
    /// 2. Wrap with `@b`: compile wrapper as `foo___b` calling `foo___impl`
    /// 3. Wrap with `@a`: compile wrapper as `foo` calling `foo___b`
    ///
    /// Annotations are applied inside-out: last annotation wraps first.
    fn compile_chained_annotations(
        &mut self,
        func_def: &FunctionDef,
        annotations: Vec<crate::bytecode::CompiledAnnotation>,
    ) -> Result<()> {
        // Step 1: Compile the raw function body as {name}___impl
        let impl_name = format!("{}___impl", func_def.name);
        let impl_def = FunctionDef {
            name: impl_name.clone(),
            name_span: func_def.name_span,
            declaring_module_path: func_def.declaring_module_path.clone(),
            doc_comment: None,
            params: func_def.params.clone(),
            return_type: func_def.return_type.clone(),
            body: func_def.body.clone(),
            type_params: func_def.type_params.clone(),
            annotations: Vec::new(),
            where_clause: None,
            is_async: func_def.is_async,
            is_comptime: func_def.is_comptime,
        };
        self.register_function(&impl_def)?;
        self.compile_function_body(&impl_def)?;

        let mut current_impl_idx =
            self.find_function(&impl_name)
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: format!("Impl function '{}' not found after compilation", impl_name),
                    location: None,
                })? as u16;

        // Step 2: Apply annotations inside-out (last annotation wraps first)
        // For @a @b @c: wrap order is c(impl) -> b(c_wrapper) -> a(b_wrapper)
        let reversed: Vec<_> = annotations.into_iter().rev().collect();
        let total = reversed.len();

        for (i, ann) in reversed.into_iter().enumerate() {
            let is_last = i == total - 1;
            let wrapper_name = if is_last {
                // The outermost annotation gets the original function name
                func_def.name.clone()
            } else {
                // Intermediate wrappers get unique names
                format!("{}___{}", func_def.name, ann.name)
            };

            // Find the annotation arg expressions from the original function def
            let ann_arg_exprs = func_def
                .annotations
                .iter()
                .find(|a| a.name == ann.name)
                .map(|a| a.args.clone())
                .unwrap_or_default();

            // Register the intermediate wrapper function (outermost already registered)
            let wrapper_func_idx = if is_last {
                self.find_function(&func_def.name)
                    .ok_or_else(|| ShapeError::RuntimeError {
                        message: format!("Function '{}' not found", func_def.name),
                        location: None,
                    })?
            } else {
                // Create a placeholder function entry for the intermediate wrapper
                let wrapper_def = FunctionDef {
                    name: wrapper_name.clone(),
                    name_span: func_def.name_span,
                    declaring_module_path: func_def.declaring_module_path.clone(),
                    doc_comment: None,
                    params: func_def.params.clone(),
                    return_type: func_def.return_type.clone(),
                    body: Vec::new(), // placeholder
                    type_params: func_def.type_params.clone(),
                    annotations: Vec::new(),
                    is_async: func_def.is_async,
                    is_comptime: func_def.is_comptime,
                    where_clause: None,
                };
                self.register_function(&wrapper_def)?;
                self.find_function(&wrapper_name)
                    .expect("function was just registered")
            };

            // Compile the wrapper that wraps current_impl_idx with self annotation
            self.compile_annotation_wrapper(
                func_def,
                wrapper_func_idx,
                current_impl_idx,
                &ann,
                &ann_arg_exprs,
            )?;

            current_impl_idx = wrapper_func_idx as u16;
        }

        Ok(())
    }

    /// Compile a function that has a single before/after annotation hook.
    ///
    /// 1. Compile original body as `{name}___impl`
    /// 2. Compile a wrapper under the original name that calls before/impl/after
    fn compile_wrapped_function(
        &mut self,
        func_def: &FunctionDef,
        compiled_ann: crate::bytecode::CompiledAnnotation,
    ) -> Result<()> {
        // Find the annotation on the function to get the arg expressions
        let ann = func_def
            .annotations
            .iter()
            .find(|a| a.name == compiled_ann.name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Annotation '{}' not found on function", compiled_ann.name),
                location: None,
            })?;
        let ann_arg_exprs = ann.args.clone();

        // Step 1: Compile original body as {name}___impl
        let impl_name = format!("{}___impl", func_def.name);
        let impl_def = FunctionDef {
            name: impl_name.clone(),
            name_span: func_def.name_span,
            declaring_module_path: func_def.declaring_module_path.clone(),
            doc_comment: None,
            params: func_def.params.clone(),
            return_type: func_def.return_type.clone(),
            body: func_def.body.clone(),
            type_params: func_def.type_params.clone(),
            annotations: Vec::new(),
            where_clause: None,
            is_async: func_def.is_async,
            is_comptime: func_def.is_comptime,
        };
        self.register_function(&impl_def)?;
        self.compile_function_body(&impl_def)?;

        let impl_idx = self
            .find_function(&impl_name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Impl function '{}' not found after compilation", impl_name),
                location: None,
            })? as u16;

        // Step 2: Compile the wrapper
        let func_idx =
            self.find_function(&func_def.name)
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: format!("Function '{}' not found", func_def.name),
                    location: None,
                })?;

        self.compile_annotation_wrapper(func_def, func_idx, impl_idx, &compiled_ann, &ann_arg_exprs)
    }

    /// Core annotation wrapper compilation.
    ///
    /// Emits bytecode for a wrapper function at `wrapper_func_idx` that:
    /// - Builds args array from function params
    /// - Calls before(self, ...ann_params, args, ctx) if present
    /// - Calls the impl function at `impl_idx` with (possibly modified) args
    /// - Calls after(self, ...ann_params, args, result, ctx) if present
    /// - Returns result
    fn compile_annotation_wrapper(
        &mut self,
        func_def: &FunctionDef,
        wrapper_func_idx: usize,
        impl_idx: u16,
        compiled_ann: &crate::bytecode::CompiledAnnotation,
        ann_arg_exprs: &[shape_ast::ast::Expr],
    ) -> Result<()> {
        let jump_over = if self.current_function.is_none() {
            Some(self.emit_jump(OpCode::Jump, 0))
        } else {
            None
        };

        let saved_function = self.current_function;
        let saved_next_local = self.next_local;
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_is_async = self.current_function_is_async;

        self.current_function = Some(wrapper_func_idx);
        self.current_function_is_async = func_def.is_async;
        self.locals = vec![HashMap::new()];
        self.type_tracker.clear_locals();
        self.push_scope();
        self.next_local = 0;

        self.program.functions[wrapper_func_idx].entry_point = self.program.current_offset();

        // Start blob builder for this wrapper function.
        let saved_blob_builder = self.current_blob_builder.take();
        let wrapper_blob_name = self.program.functions[wrapper_func_idx].name.clone();
        self.current_blob_builder = Some(super::FunctionBlobBuilder::new(
            wrapper_blob_name,
            self.program.current_offset(),
            self.program.constants.len(),
            self.program.strings.len(),
        ));

        // Bind original function params as locals
        for param in &func_def.params {
            for name in param.get_identifiers() {
                self.declare_local(&name)?;
            }
        }

        // Declare locals for wrapper internal state
        let args_local = self.declare_local("__args")?;
        let result_local = self.declare_local("__result")?;
        let ctx_local = self.declare_local("__ctx")?;

        // --- Build args array from function params ---
        // The wrapper function may have ref-inferred params (inherited from
        // the original function definition). Callers emit MakeRef for those
        // params, so local slots contain TAG_REF values. We must DerefLoad
        // to get the actual values before putting them in the args array.
        let wrapper_ref_params = self.program.functions[wrapper_func_idx].ref_params.clone();
        for (i, _param) in func_def.params.iter().enumerate() {
            if wrapper_ref_params.get(i).copied().unwrap_or(false) {
                self.emit(Instruction::new(
                    OpCode::DerefLoad,
                    Some(Operand::Local(i as u16)),
                ));
            } else {
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(i as u16)),
                ));
            }
        }
        self.emit(Instruction::new(
            OpCode::NewArray,
            Some(Operand::Count(func_def.params.len() as u16)),
        ));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(args_local)),
        ));

        // --- Build ctx object: { __impl: Function, state: {}, event_log: [] } ---
        // Push fields in schema order: __impl, state, event_log
        // __impl = reference to the implementation function
        let impl_ref_const = self
            .program
            .add_constant(Constant::Function(impl_idx as u16));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(impl_ref_const)),
        ));
        let empty_schema_id = self.type_tracker.register_inline_object_schema(&[]);
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: empty_schema_id as u16,
                field_count: 0,
            }),
        ));

        self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));

        let ctx_schema_id = self.type_tracker.register_inline_object_schema_typed(&[
            ("__impl", FieldType::Any),
            ("state", FieldType::Any),
            ("event_log", FieldType::Array(Box::new(FieldType::Any))),
        ]);
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: ctx_schema_id as u16,
                field_count: 3,
            }),
        ));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(ctx_local)),
        ));

        // --- Call before handler if present ---
        let mut short_circuit_jump: Option<usize> = None;
        if let Some(before_id) = compiled_ann.before_handler {
            let fn_ref = self
                .program
                .add_constant(Constant::Number(wrapper_func_idx as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(fn_ref)),
            ));

            for ann_arg in ann_arg_exprs {
                self.compile_expr(ann_arg)?;
            }

            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(args_local)),
            ));
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(ctx_local)),
            ));

            let before_arg_count = 1 + ann_arg_exprs.len() + 2;
            let before_ac = self
                .program
                .add_constant(Constant::Number(before_arg_count as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(before_ac)),
            ));
            self.emit(Instruction::new(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(before_id))),
            ));
            self.record_blob_call(before_id);

            let before_result = self.declare_local("__before_result")?;
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(before_result)),
            ));

            // Check if before_result is an array → replace args
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result)),
            ));
            let one_const = self.program.add_constant(Constant::Number(1.0));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(one_const)),
            ));
            self.emit(Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(crate::bytecode::BuiltinFunction::IsArray)),
            ));

            let skip_array = self.emit_jump(OpCode::JumpIfFalse, 0);

            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result)),
            ));
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(args_local)),
            ));
            let skip_obj_check = self.emit_jump(OpCode::Jump, 0);

            self.patch_jump(skip_array);

            // Check if before_result is an object → extract "args" and "state"
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result)),
            ));
            let one_const2 = self.program.add_constant(Constant::Number(1.0));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(one_const2)),
            ));
            self.emit(Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(crate::bytecode::BuiltinFunction::IsObject)),
            ));

            let skip_obj = self.emit_jump(OpCode::JumpIfFalse, 0);

            // Strict contract: before-handler object form uses typed fields
            // {args, result, state}. The `result` field enables short-circuit:
            // if the before handler returns { result: value }, skip the impl call.
            let before_contract_schema_id =
                self.type_tracker.register_inline_object_schema_typed(&[
                    ("args", FieldType::Any),
                    ("result", FieldType::Any),
                    ("state", FieldType::Any),
                ]);
            if before_contract_schema_id > u16::MAX as u32 {
                return Err(ShapeError::RuntimeError {
                    message: "Internal error: before-handler schema id overflow".to_string(),
                    location: None,
                });
            }
            let (args_operand, state_operand, result_operand) = {
                let schema = self
                    .type_tracker
                    .schema_registry()
                    .get_by_id(before_contract_schema_id)
                    .ok_or_else(|| ShapeError::RuntimeError {
                        message: "Internal error: missing before-handler schema".to_string(),
                        location: None,
                    })?;
                let args_field =
                    schema
                        .get_field("args")
                        .ok_or_else(|| ShapeError::RuntimeError {
                            message: "Internal error: before-handler schema missing 'args'"
                                .to_string(),
                            location: None,
                        })?;
                let state_field =
                    schema
                        .get_field("state")
                        .ok_or_else(|| ShapeError::RuntimeError {
                            message: "Internal error: before-handler schema missing 'state'"
                                .to_string(),
                            location: None,
                        })?;
                let result_field =
                    schema
                        .get_field("result")
                        .ok_or_else(|| ShapeError::RuntimeError {
                            message: "Internal error: before-handler schema missing 'result'"
                                .to_string(),
                            location: None,
                        })?;
                if args_field.offset > u16::MAX as usize
                    || state_field.offset > u16::MAX as usize
                    || result_field.offset > u16::MAX as usize
                {
                    return Err(ShapeError::RuntimeError {
                        message: "Internal error: before-handler field offset/index overflow"
                            .to_string(),
                        location: None,
                    });
                }
                (
                    Operand::TypedField {
                        type_id: before_contract_schema_id as u16,
                        field_idx: args_field.index as u16,
                        field_type_tag: field_type_to_tag(&args_field.field_type),
                    },
                    Operand::TypedField {
                        type_id: before_contract_schema_id as u16,
                        field_idx: state_field.index as u16,
                        field_type_tag: field_type_to_tag(&state_field.field_type),
                    },
                    Operand::TypedField {
                        type_id: before_contract_schema_id as u16,
                        field_idx: result_field.index as u16,
                        field_type_tag: field_type_to_tag(&result_field.field_type),
                    },
                )
            };

            // Check `result` field for short-circuit: if non-null, skip impl call
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result)),
            ));
            self.emit(Instruction::new(
                OpCode::GetFieldTyped,
                Some(result_operand),
            ));
            self.emit(Instruction::simple(OpCode::Dup));
            self.emit(Instruction::simple(OpCode::PushNull));
            self.emit(Instruction::simple(OpCode::Eq));
            let skip_short_circuit = self.emit_jump(OpCode::JumpIfTrue, 0);
            // result is non-null → store it and jump past impl call
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(result_local)),
            ));
            short_circuit_jump = Some(self.emit_jump(OpCode::Jump, 0));
            self.patch_jump(skip_short_circuit);
            self.emit(Instruction::simple(OpCode::Pop)); // discard null result

            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result)),
            ));
            self.emit(Instruction::new(OpCode::GetFieldTyped, Some(args_operand)));
            self.emit(Instruction::simple(OpCode::Dup));
            self.emit(Instruction::simple(OpCode::PushNull));
            self.emit(Instruction::simple(OpCode::Eq));
            let skip_args_replace = self.emit_jump(OpCode::JumpIfTrue, 0);
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(args_local)),
            ));
            let skip_pop_args = self.emit_jump(OpCode::Jump, 0);
            self.patch_jump(skip_args_replace);
            self.emit(Instruction::simple(OpCode::Pop));
            self.patch_jump(skip_pop_args);

            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result)),
            ));
            self.emit(Instruction::new(OpCode::GetFieldTyped, Some(state_operand)));
            self.emit(Instruction::simple(OpCode::Dup));
            self.emit(Instruction::simple(OpCode::PushNull));
            self.emit(Instruction::simple(OpCode::Eq));
            let skip_state = self.emit_jump(OpCode::JumpIfTrue, 0);
            self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));
            self.emit(Instruction::new(
                OpCode::NewTypedObject,
                Some(Operand::TypedObjectAlloc {
                    schema_id: ctx_schema_id as u16,
                    field_count: 2,
                }),
            ));
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(ctx_local)),
            ));
            let skip_pop_state = self.emit_jump(OpCode::Jump, 0);
            self.patch_jump(skip_state);
            self.emit(Instruction::simple(OpCode::Pop));
            self.patch_jump(skip_pop_state);

            self.patch_jump(skip_obj);
            self.patch_jump(skip_obj_check);
        }

        // --- Call impl function with (possibly modified) args ---
        // The impl function may have ref-inferred parameters (borrow inference
        // marks unannotated heap-like params as references). We must wrap those
        // args with MakeRef so the impl's DerefLoad/DerefStore opcodes find
        // TAG_REF values in the local slots.
        let impl_ref_params = self.program.functions[impl_idx as usize].ref_params.clone();
        for i in 0..func_def.params.len() {
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(args_local)),
            ));
            let idx_const = self.program.add_constant(Constant::Number(i as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(idx_const)),
            ));
            self.emit(Instruction::simple(OpCode::GetProp));
            if impl_ref_params.get(i).copied().unwrap_or(false) {
                let temp = self.declare_temp_local("__ref_wrap_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(temp)),
                ));
                self.emit(Instruction::new(
                    OpCode::MakeRef,
                    Some(Operand::Local(temp)),
                ));
            }
        }
        let impl_ac = self
            .program
            .add_constant(Constant::Number(func_def.params.len() as f64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(impl_ac)),
        ));
        self.emit(Instruction::new(
            OpCode::Call,
            Some(Operand::Function(shape_value::FunctionId(impl_idx))),
        ));
        self.record_blob_call(impl_idx);

        // For void functions, the impl returns null (the implicit return sentinel).
        // The after handler's `result` parameter would then trip the "missing
        // required argument guard" because null is the sentinel for "parameter not
        // provided". Replace null with Unit so the guard doesn't fire.
        // We only do this for explicitly void functions (return_type: Void) to avoid
        // clobbering valid return values from functions with unspecified return types.
        if compiled_ann.after_handler.is_some() {
            let is_explicit_void = matches!(
                func_def.return_type,
                Some(shape_ast::ast::TypeAnnotation::Void)
            );
            if is_explicit_void {
                // Void function: always replace null with Unit
                self.emit(Instruction::simple(OpCode::Pop));
                self.emit_unit();
            } else if func_def.return_type.is_none() {
                // Unspecified return type: replace null with Unit at runtime
                // (if the function actually returned a value, it won't be null)
                self.emit(Instruction::simple(OpCode::Dup));
                self.emit(Instruction::simple(OpCode::PushNull));
                self.emit(Instruction::simple(OpCode::Eq));
                let skip_replace = self.emit_jump(OpCode::JumpIfFalse, 0);
                // Replace the null on stack with Unit
                self.emit(Instruction::simple(OpCode::Pop));
                self.emit_unit();
                self.patch_jump(skip_replace);
            }
        }

        // Store result
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(result_local)),
        ));

        // Patch short-circuit jump: lands here, after impl call + result store
        if let Some(jump_addr) = short_circuit_jump {
            self.patch_jump(jump_addr);
        }

        // --- Call after handler if present ---
        if let Some(after_id) = compiled_ann.after_handler {
            let fn_ref = self
                .program
                .add_constant(Constant::Number(wrapper_func_idx as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(fn_ref)),
            ));

            for ann_arg in ann_arg_exprs {
                self.compile_expr(ann_arg)?;
            }

            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(args_local)),
            ));
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(result_local)),
            ));
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(ctx_local)),
            ));

            let after_arg_count = 1 + ann_arg_exprs.len() + 3;
            let after_ac = self
                .program
                .add_constant(Constant::Number(after_arg_count as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(after_ac)),
            ));
            self.emit(Instruction::new(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(after_id))),
            ));
            self.record_blob_call(after_id);

            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(result_local)),
            ));
        }

        // Return the result
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(result_local)),
        ));
        self.emit(Instruction::simple(OpCode::ReturnValue));

        // Update function locals count
        self.program.functions[wrapper_func_idx].locals_count = self.next_local;
        self.capture_function_local_storage_hints(wrapper_func_idx);

        // Finalize blob and restore the parent blob builder.
        self.finalize_current_blob(wrapper_func_idx);
        self.current_blob_builder = saved_blob_builder;

        // Restore state
        self.pop_scope();
        self.locals = saved_locals;
        self.current_function = saved_function;
        self.current_function_is_async = saved_is_async;
        self.next_local = saved_next_local;

        if let Some(jump_addr) = jump_over {
            self.patch_jump(jump_addr);
        }

        Ok(())
    }

    /// Core function body compilation (shared by normal functions and ___impl functions)
    fn compile_function_body(&mut self, func_def: &FunctionDef) -> Result<()> {
        // Find function index
        let func_idx = self
            .program
            .functions
            .iter()
            .position(|f| f.name == func_def.name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Function not found: {}", func_def.name),
                location: None,
            })?;

        // If compiling at top-level (not inside another function), emit a jump over the function body
        // This prevents the VM from falling through into function code during normal execution
        let jump_over = if self.current_function.is_none() {
            Some(self.emit_jump(OpCode::Jump, 0))
        } else {
            None
        };

        // Save current state
        let saved_function = self.current_function;
        let saved_next_local = self.next_local;
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_is_async = self.current_function_is_async;
        let saved_ref_locals = std::mem::take(&mut self.ref_locals);
        let saved_exclusive_ref_locals = std::mem::take(&mut self.exclusive_ref_locals);
        let saved_inferred_ref_locals = std::mem::take(&mut self.inferred_ref_locals);
        let saved_local_callable_pass_modes = std::mem::take(&mut self.local_callable_pass_modes);
        let saved_reference_value_locals = std::mem::take(&mut self.reference_value_locals);
        let saved_exclusive_reference_value_locals =
            std::mem::take(&mut self.exclusive_reference_value_locals);
        let saved_tracked_reference_borrow_locals =
            std::mem::take(&mut self.tracked_reference_borrow_locals);
        let saved_scoped_reference_value_locals =
            std::mem::take(&mut self.scoped_reference_value_locals);
        let saved_future_reference_use_names = std::mem::take(&mut self.future_reference_use_names);
        let saved_repeating_body_reference_local_barriers =
            std::mem::take(&mut self.repeating_body_reference_local_barriers);
        let saved_repeating_body_reference_module_binding_barriers =
            std::mem::take(&mut self.repeating_body_reference_module_binding_barriers);
        let saved_repeating_body_protected_places =
            std::mem::take(&mut self.repeating_body_protected_places);
        let saved_reference_value_module_bindings = self.reference_value_module_bindings.clone();
        let saved_exclusive_reference_value_module_bindings =
            self.exclusive_reference_value_module_bindings.clone();
        let saved_tracked_reference_borrow_module_bindings =
            self.tracked_reference_borrow_module_bindings.clone();
        let saved_scoped_reference_value_module_bindings =
            self.scoped_reference_value_module_bindings.clone();
        let saved_comptime_mode = self.comptime_mode;
        let saved_drop_locals = std::mem::take(&mut self.drop_locals);
        let saved_boxed_locals = std::mem::take(&mut self.boxed_locals);
        let saved_param_locals = std::mem::take(&mut self.param_locals);
        let saved_function_params =
            std::mem::replace(&mut self.current_function_params, func_def.params.clone());

        // Set up isolated locals for function compilation
        self.current_function = Some(func_idx);
        self.current_function_is_async = func_def.is_async;

        // If this is a `comptime fn`, mark the compilation context as comptime
        // so that calls to other `comptime fn` functions within the body are allowed.
        if func_def.is_comptime {
            self.comptime_mode = true;
        }
        self.locals = vec![HashMap::new()];
        self.type_tracker.clear_locals(); // Clear local type info for new function
        self.borrow_checker.reset(); // Reset borrow checker for new function scope
        self.ref_locals.clear();
        self.exclusive_ref_locals.clear();
        self.inferred_ref_locals.clear();
        self.local_callable_pass_modes.clear();
        self.reference_value_locals.clear();
        self.exclusive_reference_value_locals.clear();
        self.tracked_reference_borrow_locals.clear();
        self.scoped_reference_value_locals = vec![HashSet::new()];
        self.future_reference_use_names = vec![HashSet::new()];
        self.repeating_body_reference_local_barriers.clear();
        self.repeating_body_reference_module_binding_barriers
            .clear();
        self.repeating_body_protected_places.clear();
        self.immutable_locals.clear();
        self.tracked_reference_borrow_module_bindings.clear();
        self.scoped_reference_value_module_bindings = vec![HashSet::new()];
        self.param_locals.clear();
        self.push_scope();
        self.push_drop_scope();
        self.next_local = 0;

        // Reset expression-level tracking to prevent stale values from previous
        // function compilations leaking into parameter binding
        self.last_expr_schema = None;
        self.last_expr_numeric_type = None;
        self.last_expr_type_info = None;

        // Set function entry point (AFTER the jump instruction)
        self.program.functions[func_idx].entry_point = self.program.current_offset();

        // Start blob builder for this function (snapshot global pool sizes).
        let saved_blob_builder = self.current_blob_builder.take();
        self.current_blob_builder = Some(super::FunctionBlobBuilder::new(
            func_def.name.clone(),
            self.program.current_offset(),
            self.program.constants.len(),
            self.program.strings.len(),
        ));

        let inferred_modes = self.inferred_param_pass_modes.get(&func_def.name).cloned();

        // Bind parameters as locals - destructure each parameter value
        // Parameters arrive in local slots 0, 1, 2, ... from caller
        for (idx, param) in func_def.params.iter().enumerate() {
            let effective_pass_mode = inferred_modes
                .as_ref()
                .and_then(|modes| modes.get(idx))
                .copied()
                .unwrap_or_else(|| {
                    if param.is_mut_reference {
                        ParamPassMode::ByRefExclusive
                    } else if param.is_reference {
                        ParamPassMode::ByRefShared
                    } else {
                        ParamPassMode::ByValue
                    }
                });

            // Load parameter value from its slot
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(idx as u16)),
            ));
            // Destructure into bindings (self declares locals and binds them)
            self.compile_destructure_pattern(&param.pattern)?;
            self.apply_binding_semantics_to_pattern_bindings(
                &param.pattern,
                true,
                Self::binding_semantics_for_param(param, effective_pass_mode),
            );
            for (binding_name, _) in param.pattern.get_bindings() {
                if let Some(local_idx) = self.resolve_local(&binding_name) {
                    if param.is_const {
                        self.const_locals.insert(local_idx);
                        self.immutable_locals.insert(local_idx);
                    } else if matches!(effective_pass_mode, ParamPassMode::ByRefShared) {
                        self.immutable_locals.insert(local_idx);
                    }
                }
            }

            // Propagate parameter type annotations into local type tracker so
            // dot-access compiles to typed field ops (no runtime property fallback).
            if let Some(name) = param.pattern.as_identifier() {
                if let Some(local_idx) = self.resolve_local(name) {
                    if let Some(type_ann) = &param.type_annotation {
                        match type_ann {
                            shape_ast::ast::TypeAnnotation::Object(fields) => {
                                let field_refs: Vec<&str> =
                                    fields.iter().map(|f| f.name.as_str()).collect();
                                let schema_id =
                                    self.type_tracker.register_inline_object_schema(&field_refs);
                                let schema_name = self
                                    .type_tracker
                                    .schema_registry()
                                    .get_by_id(schema_id)
                                    .map(|s| s.name.clone())
                                    .unwrap_or_else(|| format!("__anon_{}", schema_id));
                                let info = crate::type_tracking::VariableTypeInfo::known(
                                    schema_id,
                                    schema_name,
                                );
                                self.type_tracker.set_local_type(local_idx, info);
                            }
                            _ => {
                                if let Some(type_name) =
                                    Self::tracked_type_name_from_annotation(type_ann)
                                {
                                    self.set_local_type_info(local_idx, &type_name);
                                }
                            }
                        }
                        self.try_track_datatable_type(type_ann, local_idx, true)?;
                    } else {
                        // Mark as a param local with inferred type (no explicit annotation).
                        // storage_hint_for_expr will not trust these for typed Add emission.
                        self.param_locals.insert(local_idx);
                        let inferred_type_name = self
                            .inferred_param_type_hints
                            .get(&func_def.name)
                            .and_then(|hints| hints.get(idx))
                            .and_then(|hint| hint.clone());
                        if let Some(type_name) = inferred_type_name {
                            self.set_local_type_info(local_idx, &type_name);
                        }
                    }
                }
            }
        }

        // Mark reference parameters in ref_locals so identifier/assignment compilation
        // emits DerefLoad/DerefStore/SetIndexRef instead of LoadLocal/StoreLocal/SetLocalIndex.
        // Also track which ref_locals were INFERRED (not explicitly declared) so that
        // closure capture can distinguish true borrows from pass-by-ref optimizations.
        for (idx, param) in func_def.params.iter().enumerate() {
            if param.is_reference {
                self.ref_locals.insert(idx as u16);
                if param.is_mut_reference {
                    self.exclusive_ref_locals.insert(idx as u16);
                }
                // A param is "inferred ref" if it has no type annotation and no explicit
                // mut reference — the compiler's pass-mode inference set is_reference.
                let was_inferred = param.type_annotation.is_none()
                    && !param.is_mut_reference
                    && inferred_modes
                        .as_ref()
                        .and_then(|modes| modes.get(idx))
                        .map_or(false, |mode| mode.is_reference());
                if was_inferred {
                    self.inferred_ref_locals.insert(idx as u16);
                }
            }
        }

        // If self is a DataTable closure, tag the first user parameter as RowView
        if let Some((schema_id, type_name)) = self.closure_row_schema.take() {
            let row_param_slot = func_def
                .params
                .first()
                .and_then(|param| param.pattern.as_identifier())
                .and_then(|name| self.resolve_local(name))
                .unwrap_or_else(|| self.program.functions[func_idx].captures_count);

            self.type_tracker.set_local_type(
                row_param_slot,
                crate::type_tracking::VariableTypeInfo::row_view(schema_id, type_name),
            );
        }

        // Parameter defaults: only check parameters that have a default value.
        // Required parameters are guaranteed to have a real value from the caller
        // (arity is enforced at call sites), so no unit-check is needed for them.
        for (idx, param) in func_def.params.iter().enumerate() {
            if let Some(default_expr) = &param.default_value {
                // Check if the caller omitted this argument (sent unit sentinel)
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(idx as u16)),
                ));
                self.emit_unit();
                self.emit(Instruction::simple(OpCode::Eq));

                let skip_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

                // Caller omitted this arg — fill in the default value
                self.compile_expr(default_expr)?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(idx as u16)),
                ));

                self.patch_jump(skip_jump);
            }
        }

        // Compile function body with implicit return support
        let body_len = func_def.body.len();
        for (idx, stmt) in func_def.body.iter().enumerate() {
            let is_last = idx == body_len - 1;

            // Check if the last statement is an expression - if so, use implicit return
            if is_last {
                match stmt {
                    Statement::Expression(expr, _) => {
                        // Compile expression and keep value on stack for implicit return
                        self.compile_expr(expr)?;
                        // Emit drops for function-level locals before returning
                        let total_scopes = self.drop_locals.len();
                        if total_scopes > 0 {
                            self.emit_drops_for_early_exit(total_scopes)?;
                        }
                        self.emit(Instruction::simple(OpCode::ReturnValue));
                        // Skip the fallback return below since we've already returned
                        // Update function locals count
                        self.program.functions[func_idx].locals_count = self.next_local;
                        self.capture_function_local_storage_hints(func_idx);
                        // Finalize blob builder and store completed blob
                        self.finalize_current_blob(func_idx);
                        self.current_blob_builder = saved_blob_builder;
                        // Restore state
                        self.drop_locals = saved_drop_locals;
                        self.boxed_locals = saved_boxed_locals;
                        self.param_locals = saved_param_locals;
                        self.current_function_params = saved_function_params;
                        self.pop_scope();
                        self.locals = saved_locals;
                        self.current_function = saved_function;
                        self.current_function_is_async = saved_is_async;
                        self.next_local = saved_next_local;
                        self.ref_locals = saved_ref_locals;
                        self.exclusive_ref_locals = saved_exclusive_ref_locals.clone();
                        self.inferred_ref_locals = saved_inferred_ref_locals.clone();
                        self.local_callable_pass_modes = saved_local_callable_pass_modes.clone();
                        self.reference_value_locals = saved_reference_value_locals;
                        self.exclusive_reference_value_locals =
                            saved_exclusive_reference_value_locals;
                        self.tracked_reference_borrow_locals =
                            saved_tracked_reference_borrow_locals;
                        self.scoped_reference_value_locals = saved_scoped_reference_value_locals;
                        self.future_reference_use_names = saved_future_reference_use_names;
                        self.repeating_body_reference_local_barriers =
                            saved_repeating_body_reference_local_barriers;
                        self.repeating_body_reference_module_binding_barriers =
                            saved_repeating_body_reference_module_binding_barriers;
                        self.repeating_body_protected_places =
                            saved_repeating_body_protected_places;
                        self.reference_value_module_bindings =
                            saved_reference_value_module_bindings;
                        self.exclusive_reference_value_module_bindings =
                            saved_exclusive_reference_value_module_bindings;
                        self.tracked_reference_borrow_module_bindings =
                            saved_tracked_reference_borrow_module_bindings;
                        self.scoped_reference_value_module_bindings =
                            saved_scoped_reference_value_module_bindings;
                        self.comptime_mode = saved_comptime_mode;
                        // Patch the jump-over instruction if we emitted one
                        if let Some(jump_addr) = jump_over {
                            self.patch_jump(jump_addr);
                        }
                        return Ok(());
                    }
                    Statement::Return(_, _) => {
                        // Explicit return - compile normally, it will handle its own return
                        let future_names = self
                            .future_reference_use_names_for_remaining_statements(
                                &func_def.body[idx + 1..],
                            );
                        self.push_future_reference_use_names(future_names);
                        let compile_result = self.compile_statement(stmt);
                        self.pop_future_reference_use_names();
                        compile_result?;
                        // After an explicit return, we still need the fallback below for
                        // control flow that might skip the return (though rare)
                    }
                    _ => {
                        // Other statement types - compile normally
                        let future_names = self
                            .future_reference_use_names_for_remaining_statements(
                                &func_def.body[idx + 1..],
                            );
                        self.push_future_reference_use_names(future_names);
                        let compile_result = self.compile_statement(stmt);
                        self.pop_future_reference_use_names();
                        compile_result?;
                        self.release_unused_local_reference_borrows_for_remaining_statements(
                            &func_def.body[idx + 1..],
                        );
                    }
                }
            } else {
                let future_names = self
                    .future_reference_use_names_for_remaining_statements(&func_def.body[idx + 1..]);
                self.push_future_reference_use_names(future_names);
                let compile_result = self.compile_statement(stmt);
                self.pop_future_reference_use_names();
                compile_result?;
                self.release_unused_local_reference_borrows_for_remaining_statements(
                    &func_def.body[idx + 1..],
                );
            }
        }

        // Emit drops for function-level locals before implicit null return
        let total_scopes = self.drop_locals.len();
        if total_scopes > 0 {
            self.emit_drops_for_early_exit(total_scopes)?;
        }

        // Implicit return null if no explicit return and last stmt wasn't an expression
        self.emit(Instruction::simple(OpCode::PushNull));
        self.emit(Instruction::simple(OpCode::ReturnValue));

        // Update function locals count
        self.program.functions[func_idx].locals_count = self.next_local;
        self.capture_function_local_storage_hints(func_idx);

        // Finalize blob builder and store completed blob
        self.finalize_current_blob(func_idx);
        self.current_blob_builder = saved_blob_builder;

        // Restore state
        self.drop_locals = saved_drop_locals;
        self.boxed_locals = saved_boxed_locals;
        self.current_function_params = saved_function_params;
        self.pop_scope();
        self.locals = saved_locals;
        self.current_function = saved_function;
        self.current_function_is_async = saved_is_async;
        self.next_local = saved_next_local;
        self.ref_locals = saved_ref_locals;
        self.exclusive_ref_locals = saved_exclusive_ref_locals;
        self.inferred_ref_locals = saved_inferred_ref_locals;
        self.local_callable_pass_modes = saved_local_callable_pass_modes;
        self.reference_value_locals = saved_reference_value_locals;
        self.exclusive_reference_value_locals = saved_exclusive_reference_value_locals;
        self.tracked_reference_borrow_locals = saved_tracked_reference_borrow_locals;
        self.scoped_reference_value_locals = saved_scoped_reference_value_locals;
        self.future_reference_use_names = saved_future_reference_use_names;
        self.repeating_body_reference_local_barriers =
            saved_repeating_body_reference_local_barriers;
        self.repeating_body_reference_module_binding_barriers =
            saved_repeating_body_reference_module_binding_barriers;
        self.repeating_body_protected_places = saved_repeating_body_protected_places;
        self.reference_value_module_bindings = saved_reference_value_module_bindings;
        self.exclusive_reference_value_module_bindings =
            saved_exclusive_reference_value_module_bindings;
        self.tracked_reference_borrow_module_bindings =
            saved_tracked_reference_borrow_module_bindings;
        self.scoped_reference_value_module_bindings = saved_scoped_reference_value_module_bindings;
        self.comptime_mode = saved_comptime_mode;

        // Patch the jump-over instruction if we emitted one
        if let Some(jump_addr) = jump_over {
            self.patch_jump(jump_addr);
        }

        Ok(())
    }

    // Compile a statement
}

#[cfg(test)]
mod tests {
    use crate::bytecode::Constant;
    use crate::compiler::{BytecodeCompiler, ParamPassMode};
    use crate::executor::{VMConfig, VirtualMachine};
    use crate::mir::analysis::BorrowErrorKind;
    use crate::type_tracking::{BindingOwnershipClass, BindingStorageClass};
    use shape_ast::ast::{DestructurePattern, FunctionParameter, Item, Span};
    use shape_value::ValueWord;

    fn eval(code: &str) -> ValueWord {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;
        let bytecode = compiler.compile(&program).expect("compile failed");
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.execute(None).expect("execution failed").clone()
    }

    fn compiles(code: &str) -> Result<crate::bytecode::BytecodeProgram, String> {
        let program =
            shape_ast::parser::parse_program(code).map_err(|e| format!("parse: {}", e))?;
        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;
        compiler
            .compile(&program)
            .map_err(|e| format!("compile: {}", e))
    }

    fn test_param(is_const: bool, is_reference: bool, is_mut_reference: bool) -> FunctionParameter {
        FunctionParameter {
            pattern: DestructurePattern::Identifier("value".to_string(), Span::DUMMY),
            is_const,
            is_reference,
            is_mut_reference,
            is_out: false,
            type_annotation: None,
            default_value: None,
        }
    }

    #[test]
    fn test_binding_semantics_for_param_modes() {
        let by_value = BytecodeCompiler::binding_semantics_for_param(
            &test_param(false, false, false),
            ParamPassMode::ByValue,
        );
        assert_eq!(
            by_value.ownership_class,
            BindingOwnershipClass::OwnedMutable
        );
        assert_eq!(by_value.storage_class, BindingStorageClass::Direct);

        let const_value = BytecodeCompiler::binding_semantics_for_param(
            &test_param(true, false, false),
            ParamPassMode::ByValue,
        );
        assert_eq!(
            const_value.ownership_class,
            BindingOwnershipClass::OwnedImmutable
        );
        assert_eq!(const_value.storage_class, BindingStorageClass::Direct);

        let shared_ref = BytecodeCompiler::binding_semantics_for_param(
            &test_param(false, true, false),
            ParamPassMode::ByRefShared,
        );
        assert_eq!(
            shared_ref.ownership_class,
            BindingOwnershipClass::OwnedImmutable
        );
        assert_eq!(shared_ref.storage_class, BindingStorageClass::Reference);

        let exclusive_ref = BytecodeCompiler::binding_semantics_for_param(
            &test_param(false, true, true),
            ParamPassMode::ByRefExclusive,
        );
        assert_eq!(
            exclusive_ref.ownership_class,
            BindingOwnershipClass::OwnedMutable
        );
        assert_eq!(exclusive_ref.storage_class, BindingStorageClass::Reference);
    }

    #[test]
    fn test_block_expr_destructured_binding_still_runs() {
        let code = r#"
            let value = {
                let [a, b] = [1, 2]
                a + b
            }
            value
        "#;
        let result = eval(code);
        assert_eq!(result.as_number_coerce().unwrap(), 3.0);
    }

    #[test]
    fn test_const_param_requires_compile_time_constant_argument() {
        let code = r#"
            function connect(const conn_str: string) {
                conn_str
            }
            let value = "duckdb://local.db"
            connect(value)
        "#;
        let err = compiles(code).expect_err("non-constant argument for const param should fail");
        assert!(
            err.contains("declared `const` and requires a compile-time constant argument"),
            "Expected const argument diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn test_const_template_skips_comptime_until_specialized() {
        let code = r#"
            annotation schema_connect() {
                comptime post(target, ctx) {
                    // `uri` is a const template parameter and is only bound on specialization.
                    if uri == "duckdb://analytics.db" {
                        set return int
                    } else {
                        set return int
                    }
                }
            }

            @schema_connect()
            function connect(const uri) {
                1
            }
        "#;
        let _ = compiles(code).expect("template base should compile without specialization");
    }

    #[test]
    fn test_const_template_specialization_binds_const_values() {
        let code = r#"
            annotation schema_connect() {
                comptime post(target, ctx) {
                    if uri == "duckdb://analytics.db" {
                        set return int
                    } else {
                        set return int
                    }
                }
            }

            @schema_connect()
            function connect(const uri) {
                1
            }

            let a = connect("duckdb://analytics.db")
            let b = connect("duckdb://other.db")
        "#;
        let bytecode = compiles(code).expect("const specialization should compile");
        let specialization_count = bytecode
            .functions
            .iter()
            .filter(|f| f.name.starts_with("connect__const_"))
            .count();
        assert_eq!(
            specialization_count, 2,
            "expected one specialization per distinct const argument"
        );
    }

    #[test]
    fn test_comptime_before_cannot_override_explicit_param_type() {
        let code = r#"
            annotation force_string() {
                comptime pre(target, ctx) {
                    set param x: string
                }
            }
            @force_string()
            function foo(x: int) {
                x
            }
        "#;
        let err = compiles(code).expect_err("explicit param type override should fail");
        assert!(
            err.contains("cannot override explicit type of parameter 'x'"),
            "Expected explicit param override error, got: {}",
            err
        );
    }

    #[test]
    fn test_comptime_after_cannot_override_explicit_return_type() {
        let code = r#"
            annotation force_string_return() {
                comptime post(target, ctx) {
                    set return string
                }
            }
            @force_string_return()
            function foo() -> int {
                1
            }
        "#;
        let err = compiles(code).expect_err("explicit return type override should fail");
        assert!(
            err.contains("cannot override explicit function return type annotation"),
            "Expected explicit return override error, got: {}",
            err
        );
    }

    #[test]
    fn test_comptime_after_receives_annotation_args() {
        let code = r#"
            annotation set_return_type_from_annotation(type_name) {
                comptime post(target, ctx, ty) {
                    if ty == "int" {
                        set return int
                    } else {
                        set return string
                    }
                }
            }
            @set_return_type_from_annotation("int")
            fn foo() {
                1
            }
            foo()
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected numeric result"),
            1.0
        );
    }

    #[test]
    fn test_comptime_after_variadic_annotation_args() {
        let code = r#"
            annotation variadic_schema() {
                comptime post(target, ctx, ...config) {
                    set return int
                }
            }
            @variadic_schema(1, "x", true)
            fn foo() {
                1
            }
            foo()
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected numeric result"),
            1.0
        );
    }

    #[test]
    fn test_comptime_after_arg_arity_errors() {
        let missing_arg = r#"
            annotation needs_arg() {
                comptime post(target, ctx, config) {
                    target.name
                }
            }
            @needs_arg()
            fn foo() { 1 }
        "#;
        let err = compiles(missing_arg).expect_err("missing annotation arg should fail");
        assert!(
            err.contains("missing annotation argument for comptime handler parameter 'config'"),
            "unexpected error: {}",
            err
        );

        let too_many = r#"
            annotation one_arg() {
                comptime post(target, ctx, config) {
                    target.name
                }
            }
            @one_arg(1, 2)
            fn foo() { 1 }
        "#;
        let err = compiles(too_many).expect_err("too many annotation args should fail");
        assert!(
            err.contains("too many annotation arguments"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_comptime_after_can_replace_function_body() {
        let code = r#"
            annotation synthesize_body() {
                comptime post(target, ctx) {
                    replace body {
                        return 42
                    }
                }
            }
            @synthesize_body()
            function foo() {
            }
            foo()
        "#;
        let result = eval(code);
        assert_eq!(
            result
                .as_number_coerce()
                .expect("Expected 42 from synthesized body"),
            42.0
        );
    }

    #[test]
    fn test_comptime_after_can_replace_function_body_from_expr() {
        let code = r#"
            comptime fn body_src() {
                "return 7"
            }

            annotation synthesize_body_expr() {
                comptime post(target, ctx) {
                    replace body (body_src())
                }
            }
            @synthesize_body_expr()
            function foo() {
            }
            foo()
        "#;
        let result = eval(code);
        assert_eq!(
            result
                .as_number_coerce()
                .expect("Expected 7 from synthesized body"),
            7.0
        );
    }

    #[test]
    fn test_comptime_handler_extend_generates_method() {
        // A comptime handler using direct `extend` should register generated methods.
        let code = r#"
            annotation add_method() {
                targets: [type]
                comptime post(target, ctx) {
                    extend Number {
                        method doubled() { self * 2.0 }
                    }
                }
            }

            @add_method()
            type Marker { x: int }

            (5.0).doubled()
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected Number(10.0)"),
            10.0
        );
    }

    #[test]
    fn test_comptime_handler_extend_method_executes() {
        // Verify the generated extend method actually runs correctly
        let code = r#"
            annotation auto_extend() {
                targets: [type]
                comptime post(target, ctx) {
                    extend Number {
                        method tripled() { self * 3.0 }
                    }
                }
            }
            @auto_extend()
            type Marker { x: int }
            (10.0).tripled()
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected Number(30.0)"),
            30.0
        );
    }

    #[test]
    fn test_comptime_handler_non_object_result_ignored() {
        // Handler values are ignored unless explicit directives are emitted.
        let code = r#"
            annotation no_op() {
                comptime post(target, ctx) {
                    "just a string"
                }
            }
            @no_op()
            function my_func(x) {
                return x + 1.0
            }
            my_func(5.0)
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected Number(6.0)"),
            6.0
        );
    }

    #[test]
    fn test_legacy_action_object_not_processed() {
        // Legacy action-object return values are intentionally ignored.
        let code = r#"
            annotation legacy() {
                comptime post(target, ctx) {
                    { action: "extend", source: "method doubled() { return self * 2.0 }", type: "Number" }
                }
            }
            @legacy()
            function placeholder() { 0 }
            (5.0).doubled()
        "#;
        let result = compiles(code).expect("legacy action object should not fail compilation");
        let has_doubled = result
            .functions
            .iter()
            .any(|f| f.name.ends_with("::doubled"));
        assert!(
            !has_doubled,
            "Legacy action-object return should not generate methods"
        );
    }

    #[test]
    fn test_comptime_handler_extend_multiple_methods() {
        // A comptime handler can emit multiple methods in one extend block.
        let code = r#"
            annotation math_ops() {
                targets: [type]
                comptime post(target, ctx) {
                    extend Number {
                        method add_ten() { self + 10.0 }
                        method sub_ten() { self - 10.0 }
                    }
                }
            }
            @math_ops()
            type Marker { x: int }
            let a = (25.0).add_ten()
            let b = (25.0).sub_ten()
            a + b
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected Number(50.0)"),
            50.0
        );
    }

    #[test]
    fn test_expression_annotation_comptime_handler_executes() {
        // Expression-level annotation should run comptime handler and process extend directives.
        let code = r#"
            annotation expr_extend() {
                targets: [expression]
                comptime post(target, ctx) {
                    extend Number {
                        method quadrupled() { self * 4.0 }
                    }
                }
            }

            let x = @expr_extend() 2.0
            x.quadrupled()
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected Number(8.0)"),
            8.0
        );
    }

    #[test]
    fn test_expression_annotation_target_validation() {
        // Type-only annotation applied to an expression should fail with a target error.
        let code = r#"
            annotation only_type() {
                targets: [type]
                comptime post(target, ctx) {
                    target.kind
                }
            }

            let x = @only_type() 1
        "#;
        let err = compiles(code).expect_err("type-only annotation on expression should fail");
        assert!(
            err.contains("cannot be applied to a expression"),
            "Expected expression target error, got: {}",
            err
        );
    }

    #[test]
    fn test_expression_annotation_rejects_definition_lifecycle_hooks() {
        let code = r#"
            annotation info() {
                metadata(target, ctx) {
                    target.kind
                }
            }

            let x = @info() 1
        "#;
        let err =
            compiles(code).expect_err("definition-time lifecycle hooks on expression should fail");
        assert!(
            err.contains("definition-time lifecycle hooks"),
            "Expected definition-time lifecycle target error, got: {}",
            err
        );
    }

    #[test]
    fn test_await_annotation_target_validation() {
        // Await-only annotation should compile in await context.
        let ok_code = r#"
            annotation only_await() {
                targets: [await_expr]
                comptime post(target, ctx) {
                    target.kind
                }
            }

            async function ready() {
                return 1
            }

            async function run() {
                await @only_await() ready()
                return 1
            }
        "#;
        assert!(
            compiles(ok_code).is_ok(),
            "await annotation should be accepted in await context"
        );

        // The same await-only annotation on a plain expression must fail.
        let bad_code = r#"
            annotation only_await() {
                targets: [await_expr]
                comptime post(target, ctx) {
                    target.kind
                }
            }

            let x = @only_await() 1
        "#;
        let err = compiles(bad_code).expect_err("await-only annotation on expression should fail");
        assert!(
            err.contains("cannot be applied to a expression"),
            "Expected expression target error, got: {}",
            err
        );
    }

    #[test]
    fn test_direct_extend_target_on_type_via_comptime_handler() {
        // Direct `extend target { ... }` should work without action-object indirection.
        let code = r#"
            annotation add_sum() {
                targets: [type]
                comptime post(target, ctx) {
                    extend target {
                        method sum() {
                            self.x + self.y
                        }
                    }
                }
            }

            @add_sum()
            type Point { x: int, y: int }

            Point { x: 2, y: 3 }.sum()
        "#;
        let result = eval(code);
        assert_eq!(result.as_number_coerce().expect("Expected 5"), 5.0);
    }

    #[test]
    fn test_direct_remove_target_on_expression() {
        // `remove target` on an expression target should replace the expression with null.
        let code = r#"
            annotation drop_expr() {
                targets: [expression]
                comptime post(target, ctx) {
                    remove target
                }
            }

            let x = @drop_expr() 123
            x
        "#;
        let result = eval(code);
        assert!(
            result.is_none(),
            "Expected None after remove target, got {:?}",
            result
        );
    }

    #[test]
    fn test_replace_body_original_calls_original_function() {
        // __original__ should call the original function body from a replacement body.
        let code = r#"
            annotation wrap() {
                comptime post(target, ctx) {
                    replace body {
                        return __original__(5) + 100
                    }
                }
            }
            @wrap()
            function add_ten(x) {
                return x + 10
            }
            add_ten(0)
        "#;
        let result = eval(code);
        assert_eq!(
            result
                .as_number_coerce()
                .expect("Expected 115 from __original__ call"),
            115.0,
        );
    }

    #[test]
    fn test_replace_body_args_contains_function_parameters() {
        // `args` should be an array of the function's parameters in the replacement body.
        let code = r#"
            annotation with_args() {
                comptime post(target, ctx) {
                    replace body {
                        return args.len()
                    }
                }
            }
            @with_args()
            function three_params(a, b, c) {
                return 0
            }
            three_params(10, 20, 30)
        "#;
        let result = eval(code);
        assert_eq!(
            result
                .as_number_coerce()
                .expect("Expected 3 from args.len()"),
            3.0,
        );
    }

    #[test]
    fn test_replace_body_original_with_no_params() {
        // __original__ should work even with zero-parameter functions.
        let code = r#"
            annotation add_one() {
                comptime post(target, ctx) {
                    replace body {
                        return __original__() + 1
                    }
                }
            }
            @add_one()
            function get_value() {
                return 41
            }
            get_value()
        "#;
        let result = eval(code);
        assert_eq!(
            result
                .as_number_coerce()
                .expect("Expected 42 from __original__() + 1"),
            42.0,
        );
    }

    #[test]
    fn test_content_addressed_program_has_main_and_functions() {
        let code = r#"
            function add(a, b) { a + b }
            function mul(a, b) { a * b }
            let x = add(2, 3)
            mul(x, 4)
        "#;
        let bytecode = compiles(code).expect("should compile");
        let ca = bytecode
            .content_addressed
            .expect("content_addressed program should be Some");

        // Should have at least __main__, add, and mul blobs
        assert!(
            ca.function_store.len() >= 3,
            "Expected at least 3 blobs (__main__, add, mul), got {}",
            ca.function_store.len()
        );

        // Entry should be set (non-zero hash)
        assert_ne!(
            ca.entry,
            crate::bytecode::FunctionHash::ZERO,
            "Entry hash should not be zero"
        );

        // Entry should be in the function store
        assert!(
            ca.function_store.contains_key(&ca.entry),
            "Entry hash should be present in function_store"
        );

        // Check that each blob has a non-zero content hash
        for (hash, blob) in &ca.function_store {
            assert_ne!(
                *hash,
                crate::bytecode::FunctionHash::ZERO,
                "Blob '{}' should have non-zero hash",
                blob.name
            );
            assert_eq!(
                *hash, blob.content_hash,
                "Blob '{}' key should match its content_hash",
                blob.name
            );
            assert!(
                !blob.instructions.is_empty(),
                "Blob '{}' should have instructions",
                blob.name
            );
        }
    }

    #[test]
    fn test_content_addressed_blob_has_local_pools() {
        let code = r#"
            function greet(name) { "hello " + name }
            greet("world")
        "#;
        let bytecode = compiles(code).expect("should compile");
        let ca = bytecode
            .content_addressed
            .expect("content_addressed program should be Some");

        // Find the greet blob
        let greet_blob = ca
            .function_store
            .values()
            .find(|b| b.name == "greet")
            .expect("greet blob should exist");

        assert_eq!(greet_blob.arity, 1);
        assert_eq!(greet_blob.param_names, vec!["name".to_string()]);
        // Should have at least one string in its local pool ("hello ")
        assert!(
            !greet_blob.strings.is_empty() || !greet_blob.constants.is_empty(),
            "greet blob should have local constants or strings"
        );
    }

    #[test]
    fn test_content_addressed_stable_hash() {
        // Compiling the same code twice should produce the same content hashes
        let code = r#"
            function double(x) { x * 2 }
            double(21)
        "#;
        let bytecode1 = compiles(code).expect("should compile");
        let bytecode2 = compiles(code).expect("should compile");

        let ca1 = bytecode1.content_addressed.expect("should have ca1");
        let ca2 = bytecode2.content_addressed.expect("should have ca2");

        // Find the double blob in both
        let double1 = ca1
            .function_store
            .values()
            .find(|b| b.name == "double")
            .expect("double blob in ca1");
        let double2 = ca2
            .function_store
            .values()
            .find(|b| b.name == "double")
            .expect("double blob in ca2");

        assert_eq!(
            double1.content_hash, double2.content_hash,
            "Same code should produce same content hash"
        );
    }

    #[test]
    fn test_extern_c_signature_supports_callback_and_nullable_cstring() {
        let code = r#"
            extern C fn walk(
                root: Option<string>,
                on_entry: (path: ptr, data: ptr) => i32
            ) -> Option<string> from "libwalk";
        "#;
        let bytecode = compiles(code).expect("should compile");
        assert_eq!(bytecode.foreign_functions.len(), 1);
        let entry = &bytecode.foreign_functions[0];
        let native = entry
            .native_abi
            .as_ref()
            .expect("extern C binding should carry native ABI metadata");
        assert_eq!(
            native.signature,
            "fn(cstring?, callback(fn(ptr, ptr) -> i32)) -> cstring?"
        );
    }

    #[test]
    fn test_extern_c_signature_maps_vec_to_native_slice() {
        let code = r#"
            extern C fn hash_bytes(data: Vec<byte>) -> u64 from "libhash";
            extern C fn split_words(data: Vec<Option<string>>) -> Vec<Option<string>> from "libhash";
        "#;
        let bytecode = compiles(code).expect("should compile");
        assert_eq!(bytecode.foreign_functions.len(), 2);
        let hash = bytecode.foreign_functions[0]
            .native_abi
            .as_ref()
            .expect("extern C function should carry native ABI metadata");
        assert_eq!(hash.signature, "fn(cslice<u8>) -> u64");
        let split = bytecode.foreign_functions[1]
            .native_abi
            .as_ref()
            .expect("extern C function should carry native ABI metadata");
        assert_eq!(split.signature, "fn(cslice<cstring?>) -> cslice<cstring?>");
    }

    #[test]
    fn test_extern_c_cmut_slice_param_marks_ref_mutate_contract() {
        let code = r#"
            extern C fn hash_bytes(data: Vec<byte>) -> u64 from "libhash";
            extern C fn mutate_bytes(data: CMutSlice<byte>) -> void from "libhash";
        "#;
        let bytecode = compiles(code).expect("should compile");
        let hash_fn = bytecode
            .functions
            .iter()
            .find(|func| func.name == "hash_bytes")
            .expect("hash_bytes function should exist");
        assert_eq!(hash_fn.ref_params, vec![false]);
        assert_eq!(hash_fn.ref_mutates, vec![false]);

        let mutate_fn = bytecode
            .functions
            .iter()
            .find(|func| func.name == "mutate_bytes")
            .expect("mutate_bytes function should exist");
        assert_eq!(mutate_fn.ref_params, vec![true]);
        assert_eq!(mutate_fn.ref_mutates, vec![true]);
    }

    #[test]
    fn test_extern_c_signature_rejects_nested_vec_type() {
        let code = r#"
            extern C fn bad(data: Vec<Vec<byte>>) -> i32 from "libbad";
        "#;
        let err = compiles(code).expect_err("nested Vec native slice should be rejected");
        assert!(err.contains("unsupported parameter type 'Vec<Vec<byte>>'"));
    }

    #[test]
    fn test_extern_c_call_targets_stub_then_call_foreign() {
        let code = r#"
            extern C fn cos_c(x: f64) -> f64 from "libm.so.6" as "cos";
            let value = cos_c(0.0)
            value
        "#;
        let bytecode = compiles(code).expect("should compile");
        let cos_idx = bytecode
            .functions
            .iter()
            .position(|f| f.name == "cos_c")
            .expect("cos_c function should exist") as u16;
        let mut saw_call_value = false;
        for ip in 0..bytecode.instructions.len() {
            let instr = bytecode.instructions[ip];
            if instr.opcode == crate::bytecode::OpCode::CallValue {
                saw_call_value = true;
            }
        }
        assert!(
            saw_call_value,
            "top-level should invoke function values through CallValue"
        );

        let cos = &bytecode.functions[cos_idx as usize];
        let stub_instrs = &bytecode.instructions[cos.entry_point..];
        assert!(
            stub_instrs
                .iter()
                .take(8)
                .any(|i| i.opcode == crate::bytecode::OpCode::CallForeign),
            "foreign stub should contain CallForeign opcode near its entry"
        );
        let ca = bytecode
            .content_addressed
            .as_ref()
            .expect("content-addressed program should exist");
        let cos_hash = *ca
            .function_store
            .iter()
            .find(|(_, blob)| blob.name == "cos_c")
            .map(|(hash, _)| hash)
            .expect("cos_c blob should exist");
        let main_blob = ca
            .function_store
            .values()
            .find(|blob| blob.name == "__main__")
            .expect("__main__ blob should exist");
        assert!(
            main_blob.dependencies.contains(&cos_hash),
            "__main__ blob must depend on cos_c hash so function constants remap correctly"
        );
        let has_dep_function_constant = main_blob
            .constants
            .iter()
            .any(|c| matches!(c, Constant::Function(0)));
        assert!(
            has_dep_function_constant,
            "__main__ constants should store function references as dependency-local indices"
        );
    }

    #[test]
    fn test_duckdb_package_style_arrow_import_compiles() {
        let code = r#"
            extern C fn duckdb_query_arrow(conn: ptr, sql: string, out_result: ptr) -> i32 from "duckdb";
            extern C fn duckdb_query_arrow_schema(result: ptr, out_schema: ptr) -> i32 from "duckdb";
            extern C fn duckdb_query_arrow_array(result: ptr, out_array: ptr) -> i32 from "duckdb";
            extern C fn duckdb_destroy_arrow(result_p: ptr) -> void from "duckdb" as "duckdb_destroy_arrow";

            type CandleRow {
                ts: i64,
                close: f64,
            }

            fn query_typed(conn: ptr, sql: string) -> Result<Table<CandleRow>, AnyError> {
                let result_cell = __native_ptr_new_cell()
                __native_ptr_write_ptr(result_cell, 0)
                duckdb_query_arrow(conn, sql, result_cell)
                let arrow_result = __native_ptr_read_ptr(result_cell)

                let schema_cell = __native_ptr_new_cell()
                __native_ptr_write_ptr(schema_cell, 0)
                duckdb_query_arrow_schema(arrow_result, schema_cell)
                let schema_handle = __native_ptr_read_ptr(schema_cell)
                let schema_ptr = __native_ptr_read_ptr(schema_handle)

                let array_cell = __native_ptr_new_cell()
                __native_ptr_write_ptr(array_cell, 0)
                duckdb_query_arrow_array(arrow_result, array_cell)
                let array_handle = __native_ptr_read_ptr(array_cell)
                let array_ptr = __native_ptr_read_ptr(array_handle)

                let typed: Result<Table<CandleRow>, AnyError> =
                    __native_table_from_arrow_c_typed(schema_ptr, array_ptr, "CandleRow")

                duckdb_destroy_arrow(result_cell)
                __native_ptr_free_cell(array_cell)
                __native_ptr_free_cell(schema_cell)
                __native_ptr_free_cell(result_cell)

                typed
            }
        "#;
        compiles(code).expect("duckdb package-style native code should compile");
    }

    #[test]
    fn test_extern_c_resolution_is_package_scoped_not_global() {
        let code = r#"
            extern C fn dep_a_call() -> i32 from "shared";
            extern C fn dep_b_call() -> i32 from "shared";
        "#;
        let mut program = shape_ast::parser::parse_program(code).expect("parse failed");
        for item in &mut program.items {
            if let shape_ast::ast::Item::ForeignFunction(def, _) = item
                && let Some(native) = def.native_abi.as_mut()
            {
                native.package_key = Some(match def.name.as_str() {
                    "dep_a_call" => "dep_a@1.0.0".to_string(),
                    "dep_b_call" => "dep_b@1.0.0".to_string(),
                    other => panic!("unexpected foreign function '{}'", other),
                });
            }
        }

        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;

        let mut resolutions = shape_runtime::native_resolution::NativeResolutionSet::default();
        resolutions.insert(shape_runtime::native_resolution::ResolvedNativeDependency {
            package_name: "dep_a".to_string(),
            package_version: "1.0.0".to_string(),
            package_key: "dep_a@1.0.0".to_string(),
            alias: "shared".to_string(),
            target: shape_runtime::project::NativeTarget::current(),
            provider: shape_runtime::project::NativeDependencyProvider::System,
            resolved_value: "libdep_a_shared.so".to_string(),
            load_target: "/tmp/libdep_a_shared.so".to_string(),
            fingerprint: "test-a".to_string(),
            declared_version: Some("1.0.0".to_string()),
            cache_key: None,
            provenance: shape_runtime::native_resolution::NativeProvenance::UpdateResolved,
        });
        resolutions.insert(shape_runtime::native_resolution::ResolvedNativeDependency {
            package_name: "dep_b".to_string(),
            package_version: "1.0.0".to_string(),
            package_key: "dep_b@1.0.0".to_string(),
            alias: "shared".to_string(),
            target: shape_runtime::project::NativeTarget::current(),
            provider: shape_runtime::project::NativeDependencyProvider::System,
            resolved_value: "libdep_b_shared.so".to_string(),
            load_target: "/tmp/libdep_b_shared.so".to_string(),
            fingerprint: "test-b".to_string(),
            declared_version: Some("1.0.0".to_string()),
            cache_key: None,
            provenance: shape_runtime::native_resolution::NativeProvenance::UpdateResolved,
        });
        compiler.native_resolution_context = Some(resolutions);

        let bytecode = compiler.compile(&program).expect("compile should succeed");
        let dep_a = bytecode.foreign_functions[0]
            .native_abi
            .as_ref()
            .expect("dep_a native ABI");
        let dep_b = bytecode.foreign_functions[1]
            .native_abi
            .as_ref()
            .expect("dep_b native ABI");

        assert_eq!(dep_a.library, "/tmp/libdep_a_shared.so");
        assert_eq!(dep_b.library, "/tmp/libdep_b_shared.so");
    }

    #[test]
    fn test_out_param_extern_c_compiles() {
        let code = r#"
            extern C fn duckdb_open(path: string, out out_db: ptr) -> i32 from "duckdb";
            extern C fn duckdb_connect(db: ptr, out out_conn: ptr) -> i32 from "duckdb";

            fn test() {
                let [status, db] = duckdb_open("test.db")
                let [s2, conn] = duckdb_connect(db)
                conn
            }
        "#;
        compiles(code).expect("out param extern C should compile");
    }

    #[test]
    fn test_out_param_void_return_single_out() {
        let code = r#"
            extern C fn duckdb_close(out db_p: ptr) -> void from "duckdb";

            fn test() {
                let db = duckdb_close()
                db
            }
        "#;
        // Single out + void return → return type is out value directly
        compiles(code).expect("single out param with void return should compile");
    }

    #[test]
    fn test_out_param_not_allowed_on_non_extern_c() {
        let code = r#"
            fn python test(out x: ptr) -> i32 { "pass" }
        "#;
        let err = compiles(code).expect_err("out params should not work on non-extern-C");
        assert!(
            err.contains("`out` parameter") && err.contains("only valid on `extern C`"),
            "Expected out-param validation error, got: {}",
            err
        );
    }

    #[test]
    fn test_out_param_must_be_ptr_type() {
        let code = r#"
            extern C fn foo(out x: i32) -> void from "lib";
        "#;
        let err = compiles(code).expect_err("out params must be ptr type");
        assert!(
            err.contains("must have type `ptr`"),
            "Expected ptr type error, got: {}",
            err
        );
    }

    #[test]
    fn test_native_builtin_blocked_from_user_code() {
        // Verify that __native_ptr_new_cell is not accessible from user code.
        let code = r#"
            fn test() {
                let cell = __native_ptr_new_cell()
                cell
            }
        "#;
        let mut compiler = BytecodeCompiler::new();
        // Do NOT set allow_internal_builtins — simulates user code
        let program = shape_ast::parser::parse_program(code).unwrap();
        let err = compiler
            .compile(&program)
            .expect_err("__native_* should be blocked from user code");
        let msg = format!("{}", err);
        assert!(
            msg.contains("Undefined function: __native_ptr_new_cell"),
            "Expected undefined function error, got: {}",
            msg
        );
    }

    #[test]
    fn test_intrinsic_builtin_blocked_from_user_code() {
        // Verify that __intrinsic_* and __json_* builtins are gated from user code.
        // Note: __into_*/__try_into_* are NOT gated (compiler-generated for type assertions).
        for intrinsic in &["__intrinsic_sum", "__intrinsic_mean", "__json_object_get"] {
            let code = format!(
                r#"
                fn test() {{
                    let x = {}([1, 2, 3])
                    x
                }}
            "#,
                intrinsic
            );
            let mut compiler = BytecodeCompiler::new();
            let program = shape_ast::parser::parse_program(&code).unwrap();
            let err = compiler
                .compile(&program)
                .expect_err(&format!("{} should be blocked from user code", intrinsic));
            let msg = format!("{}", err);
            assert!(
                msg.contains(&format!("Undefined function: {}", intrinsic)),
                "Expected undefined function error for {}, got: {}",
                intrinsic,
                msg
            );
        }
    }

    #[test]
    fn test_internal_builtin_not_unlocked_by_stdlib_name_collision() {
        let code = r#"
            type Json { payload: any }

            extend Json {
                method get(key: string) -> any {
                    __json_object_get(self.payload, key)
                }
            }
        "#;
        let mut compiler = BytecodeCompiler::new();
        compiler
            .stdlib_function_names
            .insert("Json.get".to_string());
        let program = shape_ast::parser::parse_program(code).unwrap();
        let err = compiler
            .compile(&program)
            .expect_err("user-defined Json.get must not gain __* access");
        let msg = format!("{}", err);
        assert!(
            msg.contains("Undefined function: __json_object_get"),
            "Expected undefined internal builtin error, got: {}",
            msg
        );
    }

    #[test]
    fn test_compile_function_records_mir_analysis() {
        let program = shape_ast::parser::parse_program(
            r#"
                function choose(flag, left, right) {
                    if flag { left } else { right }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("function should compile");

        let mir = compiler
            .mir_functions
            .get("choose")
            .expect("mir should be recorded");
        assert_eq!(mir.name, "choose");
        assert!(mir.num_locals >= 3, "params should appear in MIR locals");

        let analysis = compiler
            .mir_borrow_analyses
            .get("choose")
            .expect("borrow analysis should be recorded");
        assert_eq!(analysis.loans.len(), 0);
        assert!(analysis.errors.is_empty(), "analysis should be clean");
    }

    #[test]
    fn test_compile_function_records_mir_borrow_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function clash() {
                    let mut x = 1
                    let shared = &x
                    let exclusive = &mut x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR borrow conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0001"),
            "expected B0001-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("clash")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ConflictSharedExclusive),
            "expected MIR borrow conflict, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_write_while_borrowed() {
        let program = shape_ast::parser::parse_program(
            r#"
                function reassign() {
                    let mut x = 1
                    let shared = &x
                    x = 2
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR write-while-borrowed should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("reassign")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_read_while_exclusive_borrow() {
        let program = shape_ast::parser::parse_program(
            r#"
                function read_owner() {
                    let mut x = 1
                    let exclusive = &mut x
                    let copy = x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR read-while-exclusive should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0001"),
            "expected B0001-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("read_owner")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReadWhileExclusivelyBorrowed),
            "expected MIR read-while-exclusive error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function escape_ref() {
                    let x = 1
                    let r = &x
                    let alias = r
                    return alias
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR reference escape should surface as a compile error");
        assert!(
            format!("{}", err).contains("outlives its owner"),
            "expected reference-escape error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("escape_ref")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceEscape),
            "expected MIR reference-escape error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_use_after_explicit_move() {
        let program = shape_ast::parser::parse_program(
            r#"
                function moved_then_read() {
                    let x = "hi"
                    let y = move x
                    let z = x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR use-after-move should surface as a compile error");
        assert!(
            format!("{}", err).contains("after it was moved"),
            "expected use-after-move error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("moved_then_read")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::UseAfterMove),
            "expected MIR use-after-move error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_async_let_exclusive_ref_task_boundary() {
        let program = shape_ast::parser::parse_program(
            r#"
                async function spawn_conflict() {
                    let mut x = 1
                    async let fut = &mut x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR task-boundary error should surface as a compile error");
        assert!(
            format!("{}", err).contains("task boundary"),
            "expected task-boundary error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("spawn_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary),
            "expected MIR task-boundary error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_async_let_nested_task_boundary() {
        let program = shape_ast::parser::parse_program(
            r#"
                async function compute(a, &mut b, c) {
                    return a
                }
                async function spawn_nested_conflict() {
                    let mut x = 1
                    async let fut = compute(1, &mut x, 3)
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[1] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        for item in &program.items {
            if let Item::Function(func, _) = item {
                compiler
                    .register_function(func)
                    .expect("function should register");
            }
        }
        let err = compiler
            .compile_function(func)
            .expect_err("nested MIR task-boundary error should surface as a compile error");
        assert!(
            format!("{}", err).contains("task boundary"),
            "expected task-boundary error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("spawn_nested_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary),
            "expected MIR task-boundary error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_join_task_boundary() {
        let program = shape_ast::parser::parse_program(
            r#"
                async function join_conflict() {
                    let mut x = 1
                    await join all {
                        &mut x,
                        2,
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("join MIR task-boundary error should surface as a compile error");
        assert!(
            format!("{}", err).contains("task boundary"),
            "expected task-boundary error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("join_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary),
            "expected MIR task-boundary error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_closure_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function closure_escape() {
                    let x = 1
                    let r = &x
                    let f = || r
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("closure-capture reference escape should surface as a compile error");
        assert!(
            format!("{}", err).contains("[B0003]"),
            "expected B0003-style closure escape error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("closure_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceEscapeIntoClosure),
            "expected MIR closure escape error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_array_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function array_escape() {
                    let x = 1
                    let arr = [&x]
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("array reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an array"),
            "expected array-storage error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("array_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInArray),
            "expected MIR array-storage error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_indirect_array_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function indirect_array_escape() {
                    let x = 1
                    let r = &x
                    let arr = [r]
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("indirect array reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an array"),
            "expected array-storage error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("indirect_array_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInArray),
            "expected MIR indirect array-storage error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_object_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function object_escape() {
                    let x = 1
                    let obj = { value: &x }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("object reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an object or struct literal"),
            "expected object-storage error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("object_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInObject),
            "expected MIR object-storage error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_indirect_object_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function indirect_object_escape() {
                    let x = 1
                    let r = &x
                    let obj = { value: r }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("indirect object reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an object or struct literal"),
            "expected object-storage error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("indirect_object_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInObject),
            "expected MIR indirect object-storage error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_struct_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                type Point { value: int }

                function struct_escape() {
                    let x = 1
                    let point = Point { value: &x }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[1] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_item_with_context(&program.items[0], false)
            .expect("struct type should register");
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("struct reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an object or struct literal"),
            "expected struct-storage error, got {}",
            err
        );
        let analysis = compiler
            .mir_borrow_analyses
            .get("struct_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInObject),
            "expected MIR struct-storage error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_top_level_object_direct_reference_storage_rejected() {
        let program = shape_ast::parser::parse_program(
            r#"
                let x = 1
                let obj = { value: &x }
            "#,
        )
        .expect("parse failed");

        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("top-level object reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an object or struct literal"),
            "expected top-level object-storage error, got {}",
            err
        );
    }

    #[test]
    fn test_compile_top_level_struct_direct_reference_storage_rejected() {
        let program = shape_ast::parser::parse_program(
            r#"
                type Point { value: int }
                let x = 1
                let point = Point { value: &x }
            "#,
        )
        .expect("parse failed");

        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("top-level struct reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an object or struct literal"),
            "expected top-level struct-storage error, got {}",
            err
        );
    }

    #[test]
    fn test_compile_function_records_mir_enum_tuple_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                enum Maybe { Value(int), Other }

                function enum_tuple_escape() {
                    let x = 1
                    let value = Maybe::Value(&x)
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[1] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_item_with_context(&program.items[0], false)
            .expect("enum should register");
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("enum tuple reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an enum payload"),
            "expected enum-payload error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("enum_tuple_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInEnum),
            "expected MIR enum-payload error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_indirect_enum_tuple_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                enum Maybe { Value(int), Other }

                function indirect_enum_tuple_escape() {
                    let x = 1
                    let r = &x
                    let value = Maybe::Value(r)
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[1] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_item_with_context(&program.items[0], false)
            .expect("enum should register");
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("indirect enum tuple reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an enum payload"),
            "expected enum-payload error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("indirect_enum_tuple_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInEnum),
            "expected MIR indirect enum-payload error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_enum_struct_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                enum Maybe {
                    Err { code: int }
                }

                function enum_struct_escape() {
                    let x = 1
                    let value = Maybe::Err { code: &x }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[1] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_item_with_context(&program.items[0], false)
            .expect("enum should register");
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("enum struct reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an enum payload"),
            "expected enum-payload error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("enum_struct_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInEnum),
            "expected MIR enum struct-payload error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_top_level_enum_direct_reference_storage_rejected() {
        let program = shape_ast::parser::parse_program(
            r#"
                enum Maybe { Value(int), Other }
                let x = 1
                let value = Maybe::Value(&x)
            "#,
        )
        .expect("parse failed");

        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("top-level enum reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an enum payload"),
            "expected top-level enum-payload error, got {}",
            err
        );
    }

    #[test]
    fn test_compile_function_records_mir_property_assignment_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function property_assignment_escape() {
                    var obj = { value: 0 }
                    let x = 1
                    obj.value = &x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("property assignment reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an object or struct literal"),
            "expected object-field storage error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("property_assignment_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInObject),
            "expected MIR object-field reference escape error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_indirect_property_assignment_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function indirect_property_assignment_escape() {
                    var obj = { value: 0 }
                    let x = 1
                    let r = &x
                    obj.value = r
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "indirect property assignment reference storage should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("cannot store a reference in an object or struct literal"),
            "expected indirect object-field storage error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("indirect_property_assignment_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInObject),
            "expected MIR indirect object-field reference escape error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_index_assignment_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function index_assignment_escape() {
                    var arr = [0]
                    let x = 1
                    arr[0] = &x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("index assignment reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an array"),
            "expected array-element storage error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("index_assignment_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInArray),
            "expected MIR array-element reference escape error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_indirect_index_assignment_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function indirect_index_assignment_escape() {
                    var arr = [0]
                    let x = 1
                    let r = &x
                    arr[0] = r
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "indirect index assignment reference storage should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("cannot store a reference in an array"),
            "expected indirect array-element storage error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("indirect_index_assignment_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInArray),
            "expected MIR indirect array-element reference escape error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_top_level_property_assignment_direct_reference_storage_rejected() {
        let program = shape_ast::parser::parse_program(
            r#"
                let x = 1
                var obj = { value: 0 }
                obj.value = &x
            "#,
        )
        .expect("parse failed");

        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("top-level property assignment reference storage should error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an object or struct literal"),
            "expected top-level object-field storage error, got {}",
            err
        );
    }

    #[test]
    fn test_compile_top_level_index_assignment_direct_reference_storage_rejected() {
        let program = shape_ast::parser::parse_program(
            r#"
                let x = 1
                var arr = [0]
                arr[0] = &x
            "#,
        )
        .expect("parse failed");

        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("top-level index assignment reference storage should error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an array"),
            "expected top-level array-element storage error, got {}",
            err
        );
    }

    #[test]
    fn test_compile_function_records_mir_owned_closure_capture() {
        let program = shape_ast::parser::parse_program(
            r#"
                function closure_ok() {
                    let x = 1
                    let f = || x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("owned closure captures should compile cleanly");

        let analysis = compiler
            .mir_borrow_analyses
            .get("closure_ok")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "owned closure capture should stay borrow-clean, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_assignment_expr_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function nested_write() {
                    let mut x = 1
                    let shared = &x
                    let y = (x = 2)
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR assignment-expression write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("nested_write")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_if_expression_analysis() {
        let program = shape_ast::parser::parse_program(
            r#"
                function branch_write(flag) {
                    let mut x = 1
                    let shared = if flag { &x } else { &x }
                    x = 2
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("if-expression MIR lowering should stay in the supported subset");

        let analysis = compiler
            .mir_borrow_analyses
            .get("branch_write")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "simple if-expression borrow analysis should stay clean, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_while_expression_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function while_expr_conflict() {
                    let mut x = 1
                    let y = while true {
                        let shared = &x
                        x = 2
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR while-expression write conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("while_expr_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR while-expression write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_for_expression_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function for_expr_conflict(items) {
                    let mut x = 1
                    let y = for item in items {
                        let shared = &x
                        x = 2
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR for-expression write conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("for_expr_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR for-expression write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_loop_expression_break_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function loop_expr_conflict() {
                    let mut x = 1
                    let y = loop {
                        let shared = &x
                        break (x = 2)
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR loop-expression break write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("loop_expr_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR loop-expression write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_continue_expression_analysis() {
        let program = shape_ast::parser::parse_program(
            r#"
                function continue_expr(flag) {
                    let mut x = 1
                    let y = while flag {
                        if flag { continue } else { x }
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("continue inside while-expression should stay in the supported subset");

        let analysis = compiler
            .mir_borrow_analyses
            .get("continue_expr")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "continue-only while-expression analysis should stay clean, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_destructure_decl_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function destructure_decl_conflict(pair) {
                    let [left, right] = pair
                    let shared = &left
                    left = 2
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR destructuring declaration write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("destructure_decl_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR destructuring-declaration write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_destructure_param_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function destructure_param_conflict([left, right]) {
                    let shared = &left
                    left = 2
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR destructured-parameter write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("destructure_param_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR destructured-parameter write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_destructure_for_loop_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function destructure_for_conflict(items) {
                    for [left, right] in items {
                        let shared = &left
                        left = 2
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR destructuring for-loop write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("destructure_for_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR destructuring for-loop write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_match_expression_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function match_expr_conflict(flag) {
                    let mut x = 1
                    let y = match flag {
                        true => {
                            let shared = &x
                            x = 2
                        }
                        _ => 0
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR match-expression write conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("match_expr_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR match-expression write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_match_expression_identifier_guard_analysis() {
        let program = shape_ast::parser::parse_program(
            r#"
                function guarded_match(v) {
                    let y = match v {
                        x where x > 0 => x
                        _ => 0
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("simple guarded match should stay in the MIR-supported subset");

        let analysis = compiler
            .mir_borrow_analyses
            .get("guarded_match")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "guarded match analysis should stay clean, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_match_expression_array_pattern_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function array_match_conflict(pair) {
                    let mut x = 1
                    let y = match pair {
                        [left, right] => {
                            let shared = &x
                            x = 2
                        }
                        _ => 0
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR array-pattern match write conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("array_match_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR array-pattern match write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_match_expression_constructor_pattern_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function constructor_match_conflict(opt) {
                    let mut x = 1
                    let y = match opt {
                        Some(v) => {
                            let shared = &x
                            x = 2
                        }
                        None => 0
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR constructor-pattern match write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("constructor_match_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR constructor-pattern match write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_rest_destructure_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function rest_destructure_conflict(items) {
                    let [head, ...tail] = items
                    let shared = &tail
                    tail = items
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR rest-destructure write conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("rest_destructure_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR rest-destructure write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_decomposition_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function decomposition_conflict(merged) {
                    let (left: {x}, right: {y}) = merged
                    let shared = &left
                    left = merged
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR decomposition-pattern write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("decomposition_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR decomposition-pattern write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }
}
