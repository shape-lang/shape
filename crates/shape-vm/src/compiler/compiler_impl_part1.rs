use super::*;

impl BytecodeCompiler {
    pub(super) fn collect_namespace_import_bindings(program: &Program) -> Vec<String> {
        use shape_ast::ast::{ImportItems, Item};

        let mut bindings = Vec::new();
        for item in &program.items {
            if let Item::Import(import_stmt, _) = item
                && let ImportItems::Namespace { name, alias } = &import_stmt.items
            {
                bindings.push(alias.clone().unwrap_or_else(|| name.clone()));
            }
        }
        bindings
    }

    pub fn new() -> Self {
        Self {
            program: BytecodeProgram::new(),
            current_function: None,
            locals: vec![HashMap::new()],
            module_bindings: HashMap::new(),
            next_local: 0,
            next_global: 0,
            loop_stack: Vec::new(),
            closure_counter: 0,
            closure_row_schema: None,
            last_expr_type_info: None,
            type_tracker: TypeTracker::with_stdlib(),
            last_expr_schema: None,
            last_expr_numeric_type: None,
            current_expr_result_mode: ExprResultMode::Value,
            last_expr_reference_result: ExprReferenceResult::default(),
            local_callable_pass_modes: HashMap::new(),
            local_callable_return_reference_summaries: HashMap::new(),
            module_binding_callable_pass_modes: HashMap::new(),
            module_binding_callable_return_reference_summaries: HashMap::new(),
            function_return_reference_summaries: HashMap::new(),
            current_function_return_reference_summary: None,
            type_inference: shape_runtime::type_system::inference::TypeInferenceEngine::new(),
            type_aliases: HashMap::new(),
            current_line: 1,
            current_file_id: 0,
            source_text: None,
            source_lines: Vec::new(),
            imported_names: HashMap::new(),
            module_namespace_bindings: HashSet::new(),
            module_scope_stack: Vec::new(),
            known_exports: HashMap::new(),
            function_arity_bounds: HashMap::new(),
            function_const_params: HashMap::new(),
            function_defs: HashMap::new(),
            foreign_function_defs: HashMap::new(),
            const_specializations: HashMap::new(),
            next_const_specialization_id: 0,
            specialization_const_bindings: HashMap::new(),
            struct_types: HashMap::new(),
            struct_generic_info: HashMap::new(),
            native_layout_types: HashSet::new(),
            generated_native_conversion_pairs: HashSet::new(),
            current_function_is_async: false,
            source_dir: None,
            errors: Vec::new(),
            hoisted_fields: HashMap::new(),
            pending_variable_name: None,
            future_reference_use_name_scopes: Vec::new(),
            known_traits: std::collections::HashSet::new(),
            trait_defs: HashMap::new(),
            extension_registry: None,
            comptime_fields: HashMap::new(),
            type_diagnostic_mode: TypeDiagnosticMode::ReliableOnly,
            compile_diagnostic_mode: CompileDiagnosticMode::FailFast,
            comptime_mode: false,
            removed_functions: HashSet::new(),
            allow_internal_comptime_namespace: false,
            method_table: MethodTable::new(),
            ref_locals: HashSet::new(),
            exclusive_ref_locals: HashSet::new(),
            inferred_ref_locals: HashSet::new(),
            reference_value_locals: HashSet::new(),
            exclusive_reference_value_locals: HashSet::new(),
            const_locals: HashSet::new(),
            const_module_bindings: HashSet::new(),
            immutable_locals: HashSet::new(),
            param_locals: HashSet::new(),
            immutable_module_bindings: HashSet::new(),
            reference_value_module_bindings: HashSet::new(),
            exclusive_reference_value_module_bindings: HashSet::new(),
            call_arg_module_binding_ref_writebacks: Vec::new(),
            inferred_ref_params: HashMap::new(),
            inferred_ref_mutates: HashMap::new(),
            inferred_param_pass_modes: HashMap::new(),
            inferred_param_type_hints: HashMap::new(),
            drop_locals: Vec::new(),
            drop_type_info: HashMap::new(),
            drop_module_bindings: Vec::new(),
            mutable_closure_captures: HashMap::new(),
            boxed_locals: HashSet::new(),
            permission_set: None,
            current_blob_builder: None,
            completed_blobs: Vec::new(),
            blob_name_to_hash: HashMap::new(),
            content_addressed_program: None,
            function_hashes_by_id: Vec::new(),
            blob_cache: None,
            function_aliases: HashMap::new(),
            current_function_params: Vec::new(),
            stdlib_function_names: HashSet::new(),
            allow_internal_builtins: false,
            native_resolution_context: None,
            non_function_mir_context_stack: Vec::new(),
            mir_functions: HashMap::new(),
            mir_borrow_analyses: HashMap::new(),
            mir_storage_plans: HashMap::new(),
            function_borrow_summaries: HashMap::new(),
            mir_span_to_point: HashMap::new(),
            mir_field_analyses: HashMap::new(),
        }
    }

    /// Enable comptime compilation mode for this compiler instance.
    pub fn set_comptime_mode(&mut self, enabled: bool) {
        self.comptime_mode = enabled;
    }

    /// Attach a blob-level cache for incremental compilation.
    ///
    /// When set, `finalize_current_blob` stores each compiled blob in the cache,
    /// and `build_content_addressed_program` populates the function store from
    /// cached blobs when possible.
    pub fn set_blob_cache(&mut self, cache: BlobCache) {
        self.blob_cache = Some(cache);
    }

    /// Finalize the current blob builder for the function at `func_idx` and
    /// move it to `completed_blobs`. Called at the end of function body compilation.
    ///
    /// If a `blob_cache` is attached, the finalized blob is stored in the cache
    /// for reuse in subsequent compilations.
    pub(crate) fn finalize_current_blob(&mut self, func_idx: usize) {
        // Set body_length before finalizing blob
        let entry = self.program.functions[func_idx].entry_point;
        let end = self.program.instructions.len();
        self.program.functions[func_idx].body_length = end - entry;

        if let Some(builder) = self.current_blob_builder.take() {
            let instr_end = self.program.instructions.len();
            let func = &self.program.functions[func_idx];
            let blob = builder.finalize(&self.program, func, &self.blob_name_to_hash, instr_end);
            self.blob_name_to_hash
                .insert(blob.name.clone(), blob.content_hash);

            // Store in cache for future incremental compilation.
            if let Some(ref mut cache) = self.blob_cache {
                cache.put_blob(&blob);
            }

            if self.function_hashes_by_id.len() <= func_idx {
                self.function_hashes_by_id.resize(func_idx + 1, None);
            }
            self.function_hashes_by_id[func_idx] = Some(blob.content_hash);

            self.completed_blobs.push(blob);
        }
    }

    /// Record a call dependency in the current blob builder.
    /// `func_idx` is the global function index used in the `Opcode::Call` operand.
    pub(crate) fn record_blob_call(&mut self, func_idx: u16) {
        if let Some(ref mut blob) = self.current_blob_builder {
            let callee_name = self.program.functions[func_idx as usize].name.clone();
            blob.record_call(&callee_name);
        }
    }

    /// Record permissions required by a stdlib function in the current blob builder.
    /// Called during import processing so the blob captures its permission requirements.
    pub(crate) fn record_blob_permissions(&mut self, module: &str, function: &str) {
        if let Some(ref mut blob) = self.current_blob_builder {
            let perms =
                shape_runtime::stdlib::capability_tags::required_permissions(module, function);
            if !perms.is_empty() {
                blob.record_permissions(&perms);
            }
        }
    }

    /// Finalize the `__main__` blob for top-level code and assemble
    /// the content-addressed `Program` from all completed blobs.
    pub(super) fn build_content_addressed_program(&mut self) {
        use crate::bytecode::Function;

        // Finalize the __main__ blob.
        // __main__ is special: it uses the full instruction range and is not a registered Function.
        // We create a synthetic Function entry for it.
        if let Some(main_builder) = self.current_blob_builder.take() {
            let instr_end = self.program.instructions.len();

            // Create a synthetic Function for __main__
            let main_func = Function {
                name: "__main__".to_string(),
                arity: 0,
                param_names: Vec::new(),
                locals_count: self.next_local,
                entry_point: main_builder.instr_start,
                body_length: instr_end - main_builder.instr_start,
                is_closure: false,
                captures_count: 0,
                is_async: false,
                ref_params: Vec::new(),
                ref_mutates: Vec::new(),
                mutable_captures: Vec::new(),
                frame_descriptor: self.program.top_level_frame.clone(),
                osr_entry_points: Vec::new(),
            };

            let blob = main_builder.finalize(
                &self.program,
                &main_func,
                &self.blob_name_to_hash,
                instr_end,
            );
            self.blob_name_to_hash
                .insert("__main__".to_string(), blob.content_hash);
            let mut main_hash = blob.content_hash;

            // Store __main__ blob in cache.
            if let Some(ref mut cache) = self.blob_cache {
                cache.put_blob(&blob);
            }

            self.completed_blobs.push(blob);

            // Build the function_store from all completed blobs.
            let mut function_store = HashMap::new();
            for blob in &self.completed_blobs {
                function_store.insert(blob.content_hash, blob.clone());
            }

            // Fixed-point resolution of forward references.
            //
            // A single pass is insufficient: resolving B's forward deps changes B's
            // hash, which makes A's reference to B stale. We iterate until no hashes
            // change (i.e., the dependency graph reaches a fixed point).
            //
            // Mutual recursion (A calls B, B calls A) can never converge because
            // each function's hash depends on the other. We detect mutual-recursion
            // edges and treat them the same as self-recursion: use ZERO sentinel.
            // The linker resolves ZERO+callee_name to the correct function ID.

            // Build mutual-recursion edge set from callee_names.
            let mut call_edges: std::collections::HashSet<(String, String)> =
                std::collections::HashSet::new();
            for blob in function_store.values() {
                for callee in &blob.callee_names {
                    if callee != &blob.name {
                        call_edges.insert((blob.name.clone(), callee.clone()));
                    }
                }
            }
            let mut mutual_edges: std::collections::HashSet<(String, String)> =
                std::collections::HashSet::new();
            for (a, b) in &call_edges {
                if call_edges.contains(&(b.clone(), a.clone())) {
                    mutual_edges.insert((a.clone(), b.clone()));
                }
            }

            let max_iterations = 10;
            for _iteration in 0..max_iterations {
                let mut any_changed = false;
                let mut recomputed: Vec<(FunctionHash, FunctionHash, FunctionBlob)> = Vec::new();

                for blob in function_store.values() {
                    let mut updated = blob.clone();
                    let mut deps_changed = false;

                    for (i, dep) in updated.dependencies.iter_mut().enumerate() {
                        if let Some(name) = blob.callee_names.get(i) {
                            // Keep self-recursive edges as ZERO sentinel.
                            // Resolving self to a concrete hash makes the hash equation
                            // non-convergent for recursive functions.
                            if name == &blob.name {
                                continue;
                            }
                            // Keep mutual-recursion edges as ZERO sentinel.
                            // Like self-recursion, mutual recursion creates a hash
                            // equation with no fixed point. The linker resolves these
                            // using callee_names instead.
                            if mutual_edges.contains(&(blob.name.clone(), name.clone())) {
                                if *dep != FunctionHash::ZERO {
                                    *dep = FunctionHash::ZERO;
                                    deps_changed = true;
                                }
                                continue;
                            }
                            if let Some(&current) = self.blob_name_to_hash.get(name) {
                                if *dep != current {
                                    *dep = current;
                                    deps_changed = true;
                                }
                            }
                        }
                    }

                    if deps_changed {
                        let old_hash = updated.content_hash;
                        updated.finalize();
                        if updated.content_hash != old_hash {
                            recomputed.push((old_hash, updated.content_hash, updated));
                            any_changed = true;
                        }
                    }
                }

                for (old_hash, new_hash, blob) in recomputed {
                    function_store.remove(&old_hash);
                    function_store.insert(new_hash, blob.clone());
                    self.blob_name_to_hash.insert(blob.name.clone(), new_hash);
                    for slot in &mut self.function_hashes_by_id {
                        if *slot == Some(old_hash) {
                            *slot = Some(new_hash);
                        }
                    }

                    // Update cache with the re-hashed blob.
                    if let Some(ref mut cache) = self.blob_cache {
                        cache.put_blob(&blob);
                    }
                }

                if !any_changed {
                    break;
                }
            }

            // Transitive permission propagation:
            // If function A calls function B, A inherits B's required_permissions.
            // This must happen after dependency hash resolution, and forms its own
            // fixpoint because permission changes alter content hashes.
            for _perm_iter in 0..max_iterations {
                let mut perm_changed = false;
                let mut updates: Vec<(FunctionHash, shape_abi_v1::PermissionSet)> = Vec::new();

                for (hash, blob) in function_store.iter() {
                    let mut accumulated = blob.required_permissions.clone();
                    for dep_hash in &blob.dependencies {
                        if let Some(dep_blob) = function_store.get(dep_hash) {
                            let unioned = accumulated.union(&dep_blob.required_permissions);
                            if unioned != accumulated {
                                accumulated = unioned;
                            }
                        }
                    }
                    if accumulated != blob.required_permissions {
                        updates.push((*hash, accumulated));
                        perm_changed = true;
                    }
                }

                let mut rehashed: Vec<(FunctionHash, FunctionBlob)> = Vec::new();
                for (hash, perms) in updates {
                    if let Some(blob) = function_store.get_mut(&hash) {
                        blob.required_permissions = perms;
                        let old_hash = blob.content_hash;
                        blob.finalize();
                        if blob.content_hash != old_hash {
                            rehashed.push((old_hash, blob.clone()));
                        }
                    }
                }

                for (old_hash, blob) in rehashed {
                    function_store.remove(&old_hash);
                    self.blob_name_to_hash
                        .insert(blob.name.clone(), blob.content_hash);
                    for slot in &mut self.function_hashes_by_id {
                        if *slot == Some(old_hash) {
                            *slot = Some(blob.content_hash);
                        }
                    }
                    if let Some(ref mut cache) = self.blob_cache {
                        cache.put_blob(&blob);
                    }
                    function_store.insert(blob.content_hash, blob);
                }

                if !perm_changed {
                    break;
                }
            }

            // Update main_hash if it changed
            if let Some(&updated_main) = self.blob_name_to_hash.get("__main__") {
                main_hash = updated_main;
            }

            // Build module_binding_names
            let mut module_binding_names = vec![String::new(); self.module_bindings.len()];
            for (name, &idx) in &self.module_bindings {
                module_binding_names[idx as usize] = name.clone();
            }

            self.content_addressed_program = Some(ContentAddressedProgram {
                entry: main_hash,
                function_store,
                top_level_locals_count: self.next_local,
                top_level_local_storage_hints: self.program.top_level_local_storage_hints.clone(),
                module_binding_names,
                module_binding_storage_hints: self.program.module_binding_storage_hints.clone(),
                function_local_storage_hints: self.program.function_local_storage_hints.clone(),
                data_schema: self.program.data_schema.clone(),
                type_schema_registry: self.type_tracker.schema_registry().clone(),
                trait_method_symbols: self.program.trait_method_symbols.clone(),
                foreign_functions: self.program.foreign_functions.clone(),
                native_struct_layouts: self.program.native_struct_layouts.clone(),
                debug_info: self.program.debug_info.clone(),
                top_level_frame: None,
            });
        }
    }

    /// Collect top-level `comptime fn` helpers visible to nested comptime execution.
    pub(crate) fn collect_comptime_helpers(&self) -> Vec<FunctionDef> {
        let mut helpers: Vec<FunctionDef> = self
            .function_defs
            .values()
            .filter(|def| def.is_comptime)
            .cloned()
            .collect();
        helpers.sort_by(|a, b| a.name.cmp(&b.name));
        helpers
    }

    /// Register a known export for import suggestions
    ///
    /// This enables helpful error messages like:
    /// "Unknown function 'sma'. Did you mean to import from '@stdlib/finance/indicators/moving_averages'?"
    pub fn register_known_export(&mut self, function_name: &str, module_path: &str) {
        self.known_exports
            .insert(function_name.to_string(), module_path.to_string());
    }

    /// Register multiple known exports at once
    pub fn register_known_exports(&mut self, exports: &HashMap<String, String>) {
        for (name, path) in exports {
            self.known_exports.insert(name.clone(), path.clone());
        }
    }

    /// Suggest an import for an unknown function
    pub fn suggest_import(&self, function_name: &str) -> Option<&str> {
        self.known_exports.get(function_name).map(|s| s.as_str())
    }

    /// Set the source text for error messages
    pub fn set_source(&mut self, source: &str) {
        self.source_text = Some(source.to_string());
        self.source_lines = source.lines().map(|s| s.to_string()).collect();
    }

    /// Set the source text and file name for error messages
    pub fn set_source_with_file(&mut self, source: &str, file_name: &str) {
        self.source_text = Some(source.to_string());
        self.source_lines = source.lines().map(|s| s.to_string()).collect();
        // Set up the source map with this file
        self.current_file_id = self
            .program
            .debug_info
            .source_map
            .add_file(file_name.to_string());
        self.program
            .debug_info
            .source_map
            .set_source_text(self.current_file_id, source.to_string());
    }

    /// Set the current source line (from AST span)
    pub fn set_line(&mut self, line: u32) {
        self.current_line = line;
    }

    /// Set line from a Span (converts byte offset to line number)
    pub fn set_line_from_span(&mut self, span: shape_ast::ast::Span) {
        if let Some(source) = &self.source_text {
            // Count newlines up to span.start to get line number
            let line = source[..span.start.min(source.len())]
                .chars()
                .filter(|c| *c == '\n')
                .count() as u32
                + 1;
            self.current_line = line;
        }
    }

    /// Get a source line by line number (1-indexed)
    pub fn get_source_line(&self, line: usize) -> Option<&str> {
        self.source_lines
            .get(line.saturating_sub(1))
            .map(|s| s.as_str())
    }

    /// Convert a Span to a SourceLocation for error reporting
    pub(crate) fn span_to_source_location(
        &self,
        span: shape_ast::ast::Span,
    ) -> shape_ast::error::SourceLocation {
        let (line, column) = if let Some(source) = &self.source_text {
            let clamped = span.start.min(source.len());
            let line = source[..clamped].chars().filter(|c| *c == '\n').count() + 1;
            let last_nl = source[..clamped].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let column = clamped - last_nl + 1;
            (line, column)
        } else {
            (1, 1)
        };
        let source_line = self.source_lines.get(line.saturating_sub(1)).cloned();
        let mut loc = shape_ast::error::SourceLocation::new(line, column);
        if span.end > span.start {
            loc = loc.with_length(span.end - span.start);
        }
        if let Some(sl) = source_line {
            loc = loc.with_source_line(sl);
        }
        if let Some(file) = self
            .program
            .debug_info
            .source_map
            .get_file(self.current_file_id)
        {
            loc = loc.with_file(file.to_string());
        }
        loc
    }

    /// Pre-register known root-scope bindings (for REPL persistence)
    ///
    /// Call this before compilation to register bindings from previous REPL sessions.
    /// This ensures that references to these bindings compile to LoadModuleBinding/StoreModuleBinding
    /// instructions rather than causing "Undefined variable" errors.
    pub fn register_known_bindings(&mut self, names: &[String]) {
        for name in names {
            if !name.is_empty() && !self.module_bindings.contains_key(name) {
                let idx = self.next_global;
                self.module_bindings.insert(name.clone(), idx);
                self.next_global += 1;
                self.register_extension_module_schema(name);
                let module_schema_name = format!("__mod_{}", name);
                if self
                    .type_tracker
                    .schema_registry()
                    .get(&module_schema_name)
                    .is_some()
                {
                    self.set_module_binding_type_info(idx, &module_schema_name);
                    self.module_namespace_bindings.insert(name.clone());
                }
            }
        }
    }

    /// Create a new compiler with a data schema for column resolution.
    /// This enables optimized GetDataField/GetDataRow opcodes.
    pub fn with_schema(schema: crate::bytecode::DataFrameSchema) -> Self {
        let mut compiler = Self::new();
        compiler.program.data_schema = Some(schema);
        compiler
    }

    /// Set the data schema for column resolution.
    /// Must be called before compiling data access expressions.
    pub fn set_schema(&mut self, schema: crate::bytecode::DataFrameSchema) {
        self.program.data_schema = Some(schema);
    }

    /// Set the source directory for resolving relative source file paths.
    pub fn set_source_dir(&mut self, dir: std::path::PathBuf) {
        self.source_dir = Some(dir);
    }

    /// Set extension modules for comptime execution.
    pub fn with_extensions(
        mut self,
        extensions: Vec<shape_runtime::module_exports::ModuleExports>,
    ) -> Self {
        self.extension_registry = Some(Arc::new(extensions));
        self
    }

    /// Configure how shared analyzer diagnostics are emitted.
    pub fn set_type_diagnostic_mode(&mut self, mode: TypeDiagnosticMode) {
        self.type_diagnostic_mode = mode;
    }

    /// Configure expression-compilation error recovery behavior.
    pub fn set_compile_diagnostic_mode(&mut self, mode: CompileDiagnosticMode) {
        self.compile_diagnostic_mode = mode;
    }

    /// Set the active permission set for compile-time capability checking.
    ///
    /// When set, imports that require permissions not in this set will produce
    /// compile errors. Pass `None` to disable checking (default).
    pub fn set_permission_set(&mut self, permissions: Option<shape_abi_v1::PermissionSet>) {
        self.permission_set = permissions;
    }

    pub(crate) fn should_recover_compile_diagnostics(&self) -> bool {
        matches!(
            self.compile_diagnostic_mode,
            CompileDiagnosticMode::RecoverAll
        )
    }

    pub(super) fn type_error_with_location_to_shape(error: TypeErrorWithLocation) -> ShapeError {
        let mut location = SourceLocation::new(error.line.max(1), error.column.max(1));
        if let Some(file) = error.file {
            location = location.with_file(file);
        }
        if let Some(source_line) = error.source_line {
            location = location.with_source_line(source_line);
        }

        ShapeError::SemanticError {
            message: error.error.to_string(),
            location: Some(location),
        }
    }

    pub(super) fn type_errors_to_shape(errors: Vec<TypeErrorWithLocation>) -> ShapeError {
        let mut mapped: Vec<ShapeError> = errors
            .into_iter()
            .map(Self::type_error_with_location_to_shape)
            .collect();
        if mapped.len() == 1 {
            return mapped.pop().unwrap_or_else(|| ShapeError::SemanticError {
                message: "Type analysis failed".to_string(),
                location: None,
            });
        }
        ShapeError::MultiError(mapped)
    }

    pub(super) fn should_emit_type_diagnostic(error: &TypeError) -> bool {
        matches!(error, TypeError::UnknownProperty(_, _))
    }

    pub(super) fn collect_program_functions(
        program: &Program,
    ) -> HashMap<String, shape_ast::ast::FunctionDef> {
        let mut out = HashMap::new();
        Self::collect_program_functions_recursive(&program.items, None, &mut out);
        out
    }

    pub(super) fn collect_program_functions_recursive(
        items: &[shape_ast::ast::Item],
        module_prefix: Option<&str>,
        out: &mut HashMap<String, shape_ast::ast::FunctionDef>,
    ) {
        for item in items {
            match item {
                shape_ast::ast::Item::Function(func, _) => {
                    let mut qualified = func.clone();
                    if let Some(prefix) = module_prefix {
                        qualified.name = format!("{}::{}", prefix, func.name);
                    }
                    out.insert(qualified.name.clone(), qualified);
                }
                shape_ast::ast::Item::Export(export, _) => {
                    if let shape_ast::ast::ExportItem::Function(func) = &export.item {
                        let mut qualified = func.clone();
                        if let Some(prefix) = module_prefix {
                            qualified.name = format!("{}::{}", prefix, func.name);
                        }
                        out.insert(qualified.name.clone(), qualified);
                    }
                }
                shape_ast::ast::Item::Module(module_def, _) => {
                    let prefix = if let Some(parent) = module_prefix {
                        format!("{}::{}", parent, module_def.name)
                    } else {
                        module_def.name.clone()
                    };
                    Self::collect_program_functions_recursive(
                        &module_def.items,
                        Some(prefix.as_str()),
                        out,
                    );
                }
                _ => {}
            }
        }
    }

    pub(super) fn is_primitive_value_type_name(name: &str) -> bool {
        matches!(
            name,
            "int"
                | "integer"
                | "i64"
                | "number"
                | "float"
                | "f64"
                | "decimal"
                | "bool"
                | "boolean"
                | "void"
                | "unit"
                | "none"
                | "null"
                | "undefined"
                | "never"
        )
    }

    pub(super) fn annotation_is_heap_like(ann: &TypeAnnotation) -> bool {
        match ann {
            TypeAnnotation::Basic(name) => !Self::is_primitive_value_type_name(name),
            TypeAnnotation::Reference(name) => !Self::is_primitive_value_type_name(name),
            TypeAnnotation::Array(_)
            | TypeAnnotation::Tuple(_)
            | TypeAnnotation::Object(_)
            | TypeAnnotation::Function { .. }
            | TypeAnnotation::Generic { .. }
            | TypeAnnotation::Dyn(_) => true,
            TypeAnnotation::Union(types) | TypeAnnotation::Intersection(types) => {
                types.iter().any(Self::annotation_is_heap_like)
            }
            TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined => false,
        }
    }

    pub(super) fn type_is_heap_like(ty: &Type) -> bool {
        match ty {
            Type::Concrete(ann) => Self::annotation_is_heap_like(ann),
            Type::Function { .. } => false,
            Type::Generic { .. } => true,
            Type::Variable(_) | Type::Constrained { .. } => false,
        }
    }

    pub(crate) fn pass_mode_from_ref_flags(
        ref_params: &[bool],
        ref_mutates: &[bool],
        idx: usize,
    ) -> ParamPassMode {
        if !ref_params.get(idx).copied().unwrap_or(false) {
            ParamPassMode::ByValue
        } else if ref_mutates.get(idx).copied().unwrap_or(false) {
            ParamPassMode::ByRefExclusive
        } else {
            ParamPassMode::ByRefShared
        }
    }

    pub(crate) fn pass_modes_from_ref_flags(
        ref_params: &[bool],
        ref_mutates: &[bool],
    ) -> Vec<ParamPassMode> {
        let len = ref_params.len().max(ref_mutates.len());
        (0..len)
            .map(|idx| Self::pass_mode_from_ref_flags(ref_params, ref_mutates, idx))
            .collect()
    }

    pub(crate) fn build_param_pass_mode_map(
        program: &Program,
        inferred_ref_params: &HashMap<String, Vec<bool>>,
        inferred_ref_mutates: &HashMap<String, Vec<bool>>,
    ) -> HashMap<String, Vec<ParamPassMode>> {
        let funcs = Self::collect_program_functions(program);
        let mut by_function = HashMap::new();

        for (name, func) in funcs {
            let inferred_refs = inferred_ref_params.get(&name).cloned().unwrap_or_default();
            let inferred_mutates = inferred_ref_mutates.get(&name).cloned().unwrap_or_default();
            let mut modes = Vec::with_capacity(func.params.len());

            for (idx, param) in func.params.iter().enumerate() {
                let explicit_ref = param.is_reference;
                let inferred_ref = inferred_refs.get(idx).copied().unwrap_or(false);
                if !(explicit_ref || inferred_ref) {
                    modes.push(ParamPassMode::ByValue);
                    continue;
                }

                if inferred_mutates.get(idx).copied().unwrap_or(false) {
                    modes.push(ParamPassMode::ByRefExclusive);
                } else {
                    modes.push(ParamPassMode::ByRefShared);
                }
            }

            by_function.insert(name, modes);
        }

        by_function
    }
}
