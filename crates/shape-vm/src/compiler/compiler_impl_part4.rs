use super::*;

impl BytecodeCompiler {
    pub(super) fn infer_reference_model(
        program: &Program,
    ) -> (
        HashMap<String, Vec<bool>>,
        HashMap<String, Vec<bool>>,
        HashMap<String, Vec<Option<String>>>,
    ) {
        let funcs = Self::collect_program_functions(program);
        let mut inference = shape_runtime::type_system::inference::TypeInferenceEngine::new();
        let (types, _) = inference.infer_program_best_effort(program);
        let inferred_ref_params = Self::infer_reference_params_from_types(program, &types);
        let inferred_param_type_hints = Self::infer_param_type_hints_from_types(program, &types);

        let mut effective_ref_params: HashMap<String, Vec<bool>> = HashMap::new();
        for (name, func) in &funcs {
            let inferred = inferred_ref_params.get(name).cloned().unwrap_or_default();
            let mut refs = vec![false; func.params.len()];
            for (idx, param) in func.params.iter().enumerate() {
                refs[idx] = param.is_reference || inferred.get(idx).copied().unwrap_or(false);
            }
            effective_ref_params.insert(name.clone(), refs);
        }

        let mut direct_mutates: HashMap<String, Vec<bool>> = HashMap::new();
        let mut edges: Vec<(String, usize, String, usize)> = Vec::new();

        for (name, func) in &funcs {
            let caller_refs = effective_ref_params
                .get(name)
                .cloned()
                .unwrap_or_else(|| vec![false; func.params.len()]);
            let mut direct = vec![false; func.params.len()];
            let mut param_index_by_name: HashMap<String, usize> = HashMap::new();
            for (idx, param) in func.params.iter().enumerate() {
                for param_name in param.get_identifiers() {
                    param_index_by_name.insert(param_name, idx);
                }
            }
            for stmt in &func.body {
                Self::analyze_statement_for_ref_mutation(
                    stmt,
                    name,
                    &param_index_by_name,
                    &caller_refs,
                    &effective_ref_params,
                    &mut direct,
                    &mut edges,
                );
            }
            direct_mutates.insert(name.clone(), direct);
        }

        let mut result = direct_mutates;
        let mut changed = true;
        while changed {
            changed = false;
            for (caller, caller_idx, callee, callee_idx) in &edges {
                let callee_mutates = result
                    .get(callee)
                    .and_then(|flags| flags.get(*callee_idx))
                    .copied()
                    .unwrap_or(false);
                if !callee_mutates {
                    continue;
                }
                if let Some(caller_flags) = result.get_mut(caller)
                    && let Some(flag) = caller_flags.get_mut(*caller_idx)
                    && !*flag
                {
                    *flag = true;
                    changed = true;
                }
            }
        }

        (inferred_ref_params, result, inferred_param_type_hints)
    }

    pub(super) fn inferred_type_to_hint_name(ty: &Type) -> Option<String> {
        match ty {
            Type::Concrete(annotation) => Some(annotation.to_type_string()),
            Type::Generic { base, args } => {
                let base_name = Self::inferred_type_to_hint_name(base)?;
                if args.is_empty() {
                    return Some(base_name);
                }
                let mut arg_names = Vec::with_capacity(args.len());
                for arg in args {
                    arg_names.push(Self::inferred_type_to_hint_name(arg)?);
                }
                Some(format!("{}<{}>", base_name, arg_names.join(", ")))
            }
            Type::Variable(_) | Type::Constrained { .. } | Type::Function { .. } => None,
        }
    }

    pub(super) fn infer_param_type_hints_from_types(
        program: &Program,
        inferred_types: &HashMap<String, Type>,
    ) -> HashMap<String, Vec<Option<String>>> {
        let funcs = Self::collect_program_functions(program);
        let mut hints = HashMap::new();

        for (name, func) in funcs {
            let mut param_hints = vec![None; func.params.len()];
            let Some(Type::Function { params, .. }) = inferred_types.get(&name) else {
                hints.insert(name, param_hints);
                continue;
            };

            for (idx, param) in func.params.iter().enumerate() {
                if param.type_annotation.is_some() || param.simple_name().is_none() {
                    continue;
                }
                if let Some(inferred_param_ty) = params.get(idx) {
                    param_hints[idx] = Self::inferred_type_to_hint_name(inferred_param_ty);
                }
            }

            hints.insert(name, param_hints);
        }

        hints
    }

    pub(crate) fn is_definition_annotation_target(
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
    ) -> bool {
        matches!(
            target_kind,
            shape_ast::ast::functions::AnnotationTargetKind::Function
                | shape_ast::ast::functions::AnnotationTargetKind::Type
                | shape_ast::ast::functions::AnnotationTargetKind::Module
        )
    }

    /// Validate that an annotation is applicable to the requested target kind.
    pub(crate) fn validate_annotation_target_usage(
        &self,
        ann: &shape_ast::ast::Annotation,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
        fallback_span: shape_ast::ast::Span,
    ) -> Result<()> {
        let Some(compiled) = self.program.compiled_annotations.get(&ann.name) else {
            let span = if ann.span == shape_ast::ast::Span::DUMMY {
                fallback_span
            } else {
                ann.span
            };
            return Err(ShapeError::SemanticError {
                message: format!("Unknown annotation '@{}'", ann.name),
                location: Some(self.span_to_source_location(span)),
            });
        };

        let has_definition_lifecycle =
            compiled.on_define_handler.is_some() || compiled.metadata_handler.is_some();
        if has_definition_lifecycle && !Self::is_definition_annotation_target(target_kind) {
            let target_label = format!("{:?}", target_kind).to_lowercase();
            let span = if ann.span == shape_ast::ast::Span::DUMMY {
                fallback_span
            } else {
                ann.span
            };
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Annotation '{}' defines definition-time lifecycle hooks (`on_define`/`metadata`) and cannot be applied to a {}. Allowed targets for these hooks are: function, type, module",
                    ann.name, target_label
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        if compiled.allowed_targets.is_empty() || compiled.allowed_targets.contains(&target_kind) {
            return Ok(());
        }

        let allowed: Vec<String> = compiled
            .allowed_targets
            .iter()
            .map(|k| format!("{:?}", k).to_lowercase())
            .collect();
        let target_label = format!("{:?}", target_kind).to_lowercase();

        let span = if ann.span == shape_ast::ast::Span::DUMMY {
            fallback_span
        } else {
            ann.span
        };

        Err(ShapeError::SemanticError {
            message: format!(
                "Annotation '{}' cannot be applied to a {}. Allowed targets: {}",
                ann.name,
                target_label,
                allowed.join(", ")
            ),
            location: Some(self.span_to_source_location(span)),
        })
    }

    /// Compile a program to bytecode
    pub fn compile(mut self, program: &Program) -> Result<BytecodeProgram> {
        // First: desugar the program (converts FromQuery to method chains, etc.)
        let mut program = program.clone();
        shape_ast::transform::desugar_program(&mut program);
        let analysis_program =
            shape_ast::transform::augment_program_with_generated_extends(&program);

        // Run the shared analyzer and surface diagnostics that are currently
        // proven reliable in the compiler execution path.
        let mut known_bindings: Vec<String> = self.module_bindings.keys().cloned().collect();
        let namespace_bindings = Self::collect_namespace_import_bindings(&analysis_program);
        known_bindings.extend(namespace_bindings.iter().cloned());
        self.module_namespace_bindings
            .extend(namespace_bindings.into_iter());
        // Auto-register extension module names as implicit namespace bindings
        // so that `regex.is_match(...)` works without a `use regex` statement.
        if let Some(ref registry) = self.extension_registry {
            for ext in registry.iter() {
                if !self.module_namespace_bindings.contains(&ext.name) {
                    self.module_namespace_bindings.insert(ext.name.clone());
                    known_bindings.push(ext.name.clone());
                }
            }
        }
        for namespace in self.module_namespace_bindings.clone() {
            let binding_idx = self.get_or_create_module_binding(&namespace);
            self.register_extension_module_schema(&namespace);
            let module_schema_name = format!("__mod_{}", namespace);
            if self
                .type_tracker
                .schema_registry()
                .get(&module_schema_name)
                .is_some()
            {
                self.set_module_binding_type_info(binding_idx, &module_schema_name);
            }
        }
        known_bindings.sort();
        known_bindings.dedup();
        let analysis_mode = if matches!(self.type_diagnostic_mode, TypeDiagnosticMode::RecoverAll) {
            TypeAnalysisMode::RecoverAll
        } else {
            TypeAnalysisMode::FailFast
        };
        if let Err(errors) = analyze_program_with_mode(
            &analysis_program,
            self.source_text.as_deref(),
            None,
            Some(&known_bindings),
            analysis_mode,
        ) {
            match self.type_diagnostic_mode {
                TypeDiagnosticMode::Strict => {
                    return Err(Self::type_errors_to_shape(errors));
                }
                TypeDiagnosticMode::ReliableOnly => {
                    let strict_errors: Vec<_> = errors
                        .into_iter()
                        .filter(|error| Self::should_emit_type_diagnostic(&error.error))
                        .collect();
                    if !strict_errors.is_empty() {
                        return Err(Self::type_errors_to_shape(strict_errors));
                    }
                }
                TypeDiagnosticMode::RecoverAll => {
                    self.errors.extend(
                        errors
                            .into_iter()
                            .map(Self::type_error_with_location_to_shape),
                    );
                }
            }
        }

        let (inferred_ref_params, inferred_ref_mutates, inferred_param_type_hints) =
            Self::infer_reference_model(&program);
        self.inferred_param_pass_modes =
            Self::build_param_pass_mode_map(&program, &inferred_ref_params, &inferred_ref_mutates);
        self.inferred_ref_params = inferred_ref_params;
        self.inferred_ref_mutates = inferred_ref_mutates;
        self.inferred_param_type_hints = inferred_param_type_hints;

        // Two-phase TypedObject field hoisting:
        //
        // Phase 1 (here, AST pre-pass): Collect all property assignments (e.g.,
        // `a.y = 2`) from the entire program BEFORE any function compilation.
        // This populates `hoisted_fields` so that `compile_typed_object_literal`
        // can allocate schema slots for future fields at object-creation time.
        // Without this pre-pass, the schema would be too small and a later
        // `a.y = 2` would require a schema migration at runtime.
        //
        // Phase 2 (per-function, MIR): During function compilation, MIR field
        // analysis (`mir::field_analysis::analyze_fields`) runs flow-sensitive
        // definite-initialization and liveness analysis. This detects:
        //   - `dead_fields`: fields that are written but never read (wasted slots)
        //   - `conditionally_initialized`: fields only assigned on some paths
        //
        // After MIR analysis, the compiler can cross-reference
        // `mir_field_analyses[func].dead_fields` to prune unused hoisted fields
        // from schemas. The dead_fields set uses `(SlotId, FieldIdx)` which must
        // be mapped to field names via the schema registry — see the integration
        // note in `compile_typed_object_literal`.
        {
            use shape_runtime::type_system::inference::PropertyAssignmentCollector;
            let assignments = PropertyAssignmentCollector::collect(&program);
            let grouped = PropertyAssignmentCollector::group_by_variable(&assignments);
            for (var_name, var_assignments) in grouped {
                let field_names: Vec<String> =
                    var_assignments.iter().map(|a| a.property.clone()).collect();
                self.hoisted_fields.insert(var_name, field_names);
            }
        }

        // First pass: collect all function definitions
        for item in &program.items {
            self.register_item_functions(item)?;
        }

        // MIR authority for non-function items: run borrow analysis on top-level
        // code before compilation. Errors in cleanly-lowered regions are emitted;
        // errors in fallback regions are suppressed (span-granular filtering).
        if let Err(e) = self.analyze_non_function_items_with_mir("__main__", &program.items) {
            self.errors.push(e);
        }

        // Start __main__ blob builder for top-level code.
        self.current_blob_builder = Some(FunctionBlobBuilder::new(
            "__main__".to_string(),
            self.program.current_offset(),
            self.program.constants.len(),
            self.program.strings.len(),
        ));

        // Push a top-level drop scope so that block expressions and
        // statement-level VarDecls can track locals for auto-drop.
        self.push_drop_scope();
        self.non_function_mir_context_stack
            .push("__main__".to_string());

        // Second pass: compile all items (collect errors instead of early-returning)
        let item_count = program.items.len();
        for (idx, item) in program.items.iter().enumerate() {
            let is_last = idx == item_count - 1;
            let future_names =
                self.future_reference_use_names_for_remaining_items(&program.items[idx + 1..]);
            self.push_future_reference_use_names(future_names);
            let compile_result = self.compile_item_with_context(item, is_last);
            self.pop_future_reference_use_names();
            if let Err(e) = compile_result {
                self.errors.push(e);
            }
            self.release_unused_module_reference_borrows_for_remaining_items(
                &program.items[idx + 1..],
            );
        }
        self.non_function_mir_context_stack.pop();

        // Return collected errors before emitting Halt
        if !self.errors.is_empty() {
            if self.errors.len() == 1 {
                return Err(self.errors.remove(0));
            }
            return Err(shape_ast::error::ShapeError::MultiError(self.errors));
        }

        // Emit drops for top-level locals (from the top-level drop scope)
        self.pop_drop_scope()?;

        // Emit drops for top-level module bindings that have Drop impls
        {
            let bindings: Vec<(u16, bool)> = std::mem::take(&mut self.drop_module_bindings);
            for (binding_idx, is_async) in bindings.into_iter().rev() {
                self.emit_drop_call_for_module_binding(binding_idx, is_async);
            }
        }

        // Add halt instruction at the end
        self.emit(Instruction::simple(OpCode::Halt));

        // Store module_binding variable names for REPL persistence
        // Build a Vec<String> where index matches the module_binding variable index
        let mut module_binding_names = vec![String::new(); self.module_bindings.len()];
        for (name, &idx) in &self.module_bindings {
            module_binding_names[idx as usize] = name.clone();
        }
        self.program.module_binding_names = module_binding_names;

        // Store top-level locals count so executor can advance sp past them
        self.program.top_level_locals_count = self.next_local;

        // Persist storage hints for JIT width-aware lowering.
        self.populate_program_storage_hints();

        // Transfer type schema registry for TypedObject field resolution
        self.program.type_schema_registry = self.type_tracker.schema_registry().clone();

        // Transfer final function definitions after comptime mutation/specialization.
        self.program.expanded_function_defs = self.function_defs.clone();

        // Finalize the __main__ blob and build the content-addressed program.
        self.build_content_addressed_program();

        // Transfer content-addressed program to the bytecode output.
        self.program.content_addressed = self.content_addressed_program.take();
        if self.program.functions.is_empty() {
            self.program.function_blob_hashes.clear();
        } else {
            if self.function_hashes_by_id.len() < self.program.functions.len() {
                self.function_hashes_by_id
                    .resize(self.program.functions.len(), None);
            } else if self.function_hashes_by_id.len() > self.program.functions.len() {
                self.function_hashes_by_id
                    .truncate(self.program.functions.len());
            }
            self.program.function_blob_hashes = self.function_hashes_by_id.clone();
        }

        // Transfer source text for error messages
        if let Some(source) = self.source_text {
            // Set in legacy field for backward compatibility
            self.program.debug_info.source_text = source.clone();
            // Also set in source map if not already set
            if self.program.debug_info.source_map.files.is_empty() {
                self.program
                    .debug_info
                    .source_map
                    .add_file("<main>".to_string());
            }
            if self.program.debug_info.source_map.source_texts.is_empty() {
                self.program
                    .debug_info
                    .source_map
                    .set_source_text(0, source);
            }
        }

        Ok(self.program)
    }

    /// Compile a program to bytecode with source text for error messages
    pub fn compile_with_source(
        mut self,
        program: &Program,
        source: &str,
    ) -> Result<BytecodeProgram> {
        self.set_source(source);
        self.compile(program)
    }

    /// Compile an imported module's AST to a standalone BytecodeProgram.
    ///
    /// This takes the Module's AST (Program), compiles all exported functions
    /// to bytecode, and returns the compiled program along with a mapping of
    /// exported function names to their function indices in the compiled output.
    ///
    /// The returned `BytecodeProgram` and function name mapping allow the import
    /// handler to resolve imported function calls to the correct bytecode indices.
    ///
    /// Currently handles function exports only. Types and values can be added later.
    pub fn compile_module_ast(
        module_ast: &Program,
    ) -> Result<(BytecodeProgram, HashMap<String, usize>)> {
        let mut compiler = BytecodeCompiler::new();
        // Stdlib modules need access to __* builtins (intrinsics, into, etc.)
        compiler.allow_internal_builtins = true;
        let bytecode = compiler.compile(module_ast)?;

        // Build name → function index mapping for exported functions
        let mut export_map = HashMap::new();
        for (idx, func) in bytecode.functions.iter().enumerate() {
            export_map.insert(func.name.clone(), idx);
        }

        Ok((bytecode, export_map))
    }
}
