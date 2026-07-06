//! Foreign function (extern C) compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use shape_ast::ast::FunctionDef;
use shape_ast::error::{Result, ShapeError};

use super::BytecodeCompiler;

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

        // ffi-rebuild §4.10 S2 / OQ4: warn on the deprecated CamelCase
        // C-view/slice spellings; the lowercase `cview`/`cmut`/`cslice`/
        // `cmut_slice` forms are canonical (they still compile for one release).
        if def.is_native_abi() {
            self.warn_deprecated_ctype_aliases(def);
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
                // A7 (integration §4.1 / §4.4.3): store the DECLARED alias, not
                // the compile-host-resolved soname. Resolution to a concrete
                // path/soname is deferred wholly to the executing host
                // (link-now locally, load-verify on remote/resume) so identical
                // declarations hash identically across compile hosts —
                // `compute_content_hash` (`core_types.rs`) covers this string.
                library: native.library.clone(),
                symbol: native.symbol.clone(),
                signature,
            })
        } else {
            None
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
            // A5(ii): filled in below from the hash-derived schema name.
            return_type_schema_id: None,
            content_hash: None,
            native_abi,
        };
        // The content hash excludes `return_type_schema_id` (a registry-local
        // numeric id that never crosses node boundaries), so it is final here
        // and can seed the hash-derived schema name below.
        entry.compute_content_hash();

        // A5(ii) (integration §4.2.6): register the anonymous foreign-return
        // schema under a HASH-DERIVED name `__ffi_h{hex16}_return`, where
        // `hex16` is the first 16 hex chars of the entry's content hash — not
        // the function name. Keying by content hash makes the receiver's
        // `return_type_schema_id` re-stamp immune to name dedup / name
        // tampering and correctly shares one return schema between two aliases
        // of one body (identical `return_type` ⇒ identical hash). Neutral to
        // every existing hash: the schema name reaches no hashed payload.
        let return_type_schema_id = if def.is_native_abi() {
            None
        } else {
            let hex16: String = entry
                .content_hash
                .map(|h| h[..8].iter().map(|b| format!("{b:02x}")).collect())
                .unwrap_or_default();
            let schema_name = format!("__ffi_h{hex16}_return");
            def.return_type
                .as_ref()
                .and_then(|ann| Self::find_object_in_annotation(ann))
                .map(|obj_fields| {
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
        entry.return_type_schema_id = return_type_schema_id;
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
            let arg_count_const = self.program.add_constant(Constant::Int(arg_count as i64));
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

        // ffi-rebuild §4.8.3 — compile-time `Ffi` derivation. Every foreign
        // call requires `Permission::Ffi`. Stamp it into the extern-stub blob's
        // `required_permissions` (the blob that carries the `CallForeign`
        // instruction) so that:
        //   (a) the linker's transitive union propagates `Ffi` to every blob
        //       that can reach a foreign call (`linker.rs` folds each blob's
        //       `required_permissions`), and
        //   (b) it is baked into the blob's content hash — two otherwise-
        //       identical programs, one calling foreign code, hash differently,
        //       so load-time capability checking and the Deterministic-mode
        //       load-time refusal (§4.8.3, both keyed on
        //       `total_required_permissions ∋ Ffi`) work with zero runtime cost.
        // This is the derivation the WF-1D `Permission::Ffi` reservation
        // deferred to WF-2A (`shape-abi-v1/src/lib.rs` variant note).
        if let Some(ref mut blob) = self.current_blob_builder {
            blob.record_permissions(&shape_abi_v1::PermissionSet::from([
                shape_abi_v1::Permission::Ffi,
            ]));
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
                self.lookup_compiled_annotation(ann)
                    .map(|(_, compiled)| compiled)
                    .filter(|c| c.before_handler.is_some() || c.after_handler.is_some())
            })
            .collect();

        if let Some(compiled_ann) = foreign_annotations.into_iter().next() {
            let ann_arg_exprs =
                self.annotation_args_for_compiled_name(&def.annotations, &compiled_ann.name);

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
                mir_data: None,
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
                let argc_const = self.program.add_constant(Constant::Int($argc as i64));
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
            .add_constant(Constant::Int(total_c_args as i64));
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
        //
        // `void` may parse to either `TypeAnnotation::Void` or the textual
        // `Basic("void")` depending on grammar context (both are accepted by
        // `native_ctype_from_annotation`); detect both. Getting this right
        // matters: a single `out` param + void return returns the out value
        // DIRECTLY, avoiding a 2-tuple `NewArray` (the DuckDB
        // `duckdb_open(path, out db)` shape and the book's out-param example).
        let is_void_return = def.return_type.as_ref().map_or(false, |ann| {
            matches!(ann, shape_ast::ast::TypeAnnotation::Void)
                || matches!(ann, shape_ast::ast::TypeAnnotation::Basic(n) if n == "void")
        });

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

    /// Emit a one-release deprecation warning for each parameter/return that
    /// uses a CamelCase C-view/slice spelling (`CView`/`CMut`/`CSlice`/
    /// `CMutSlice`). The canonical lowercase spellings (`cview`/`cmut`/
    /// `cslice`/`cmut_slice`) are the book's; both compile today (ffi-rebuild
    /// §4.10 S2 / OQ4), but the CamelCase aliases are scheduled for removal.
    fn warn_deprecated_ctype_aliases(&self, def: &shape_ast::ast::ForeignFunctionDef) {
        let mut seen: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
        let mut annotations: Vec<&shape_ast::ast::TypeAnnotation> = def
            .params
            .iter()
            .filter_map(|p| p.type_annotation.as_ref())
            .collect();
        if let Some(ret) = def.return_type.as_ref() {
            annotations.push(ret);
        }
        for ann in annotations {
            if let Some((deprecated, canonical)) = Self::deprecated_ctype_alias_in(ann)
                && seen.insert(deprecated)
            {
                self.emit_foreign_warning(
                    format!(
                        "`{deprecated}<...>` is a deprecated spelling; use the canonical \
                         `{canonical}<...>` instead (extern C function '{}')",
                        def.name
                    ),
                    def.name_span,
                );
            }
        }
    }

    /// Recursively locate the first deprecated CamelCase C-view/slice spelling
    /// in an annotation tree, returning `(deprecated, canonical)` spellings.
    fn deprecated_ctype_alias_in(
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<(&'static str, &'static str)> {
        use shape_ast::ast::TypeAnnotation;
        match ann {
            TypeAnnotation::Generic { name, args } => {
                let hit = match name.as_str() {
                    "CView" => Some(("CView", "cview")),
                    "CMut" => Some(("CMut", "cmut")),
                    "CSlice" => Some(("CSlice", "cslice")),
                    "CMutSlice" => Some(("CMutSlice", "cmut_slice")),
                    _ => None,
                };
                if hit.is_some() {
                    return hit;
                }
                args.iter().find_map(Self::deprecated_ctype_alias_in)
            }
            TypeAnnotation::Array(inner) => Self::deprecated_ctype_alias_in(inner),
            TypeAnnotation::Function { params, returns } => params
                .iter()
                .find_map(|p| Self::deprecated_ctype_alias_in(&p.type_annotation))
                .or_else(|| Self::deprecated_ctype_alias_in(returns)),
            _ => None,
        }
    }

    /// Emit a spanned, LSDS-routed compile-time warning on the foreign-function
    /// diagnostic channel. Mirrors `surface_comptime_warnings`' terminal render.
    fn emit_foreign_warning(&self, message: String, span: shape_ast::ast::Span) {
        let sl = self.span_to_source_location(span);
        let loc = shape_diagnostics::Location::new(
            sl.file.clone(),
            sl.line as u32,
            sl.column as u32,
            span.start as u32,
            span.end as u32,
        );
        let diag = shape_diagnostics::DiagnosticBuilder::new(
            "F0001",
            shape_diagnostics::Severity::Warning,
            loc,
            message,
        )
        .build();
        eprintln!(
            "{}",
            shape_diagnostics::render::terminal::render(&diag).trim_end()
        );
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
            TypeAnnotation::Basic(name) => match name.as_str() {
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
            TypeAnnotation::Reference(name) => match name.as_str() {
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
            // Slice carriers. Canonical (book, ffi-rebuild §4.10 S2 / OQ4):
            // lowercase `cslice<T>` / `cmut_slice<T>`. `Vec<T>` sugars to
            // `cslice<T>`. The CamelCase `CSlice`/`CMutSlice` are accepted as
            // one-release deprecated aliases (warned in `compile_foreign_function`).
            TypeAnnotation::Generic { name, args }
                if (name == "Vec"
                    || name == "cslice"
                    || name == "cmut_slice"
                    || name == "CSlice"
                    || name == "CMutSlice")
                    && args.len() == 1 =>
            {
                let elem = Self::native_slice_elem_ctype_from_annotation(&args[0])?;
                if name == "cmut_slice" || name == "CMutSlice" {
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
            // Pointer-backed struct views. Canonical (book, ffi-rebuild §4.10
            // S2 / OQ4): lowercase `cview<T>` (read) / `cmut<T>` (mutable). The
            // CamelCase `CView`/`CMut` are accepted as one-release deprecated
            // aliases (warned in `compile_foreign_function`).
            TypeAnnotation::Generic { name, args }
                if (name == "cview" || name == "cmut" || name == "CView" || name == "CMut")
                    && args.len() == 1 =>
            {
                let inner = match &args[0] {
                    TypeAnnotation::Basic(type_name) => type_name.clone(),
                    TypeAnnotation::Reference(type_name) => type_name.to_string(),
                    _ => return None,
                };
                if name == "cview" || name == "CView" {
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

    // A7 (integration §4.1 / §4.4.3): the compile side no longer resolves the
    // native-library alias into the entry (it stores the declared alias so
    // hashes are host-stable). This resolution chain is retained as the
    // template the executing host reuses at link-now / load-verify — the
    // design names `functions_foreign.rs:855-896` as the mirror for the
    // receiver-side resolution chain. Wired into the link-now path in WF-2A
    // stage 1+; kept here to avoid re-deriving it.
    #[allow(dead_code)]
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
}

#[cfg(test)]
mod ffi_permission_tests {
    //! ffi-rebuild stage 4a — `Permission::Ffi` enforcement (§4.8).
    //!
    //! Default-tier (NOT `deep-tests`-gated) so foreign permission gating is
    //! part of the never-die sentinel: compile-time `Ffi` derivation, the
    //! two-phase call gate (coarse `Ffi` before link, library/symbol scope
    //! before dlopen), the OQ13 local-unscoped posture, and the Q6
    //! Deterministic load-time refusal.
    use crate::compiler::BytecodeCompiler;
    use crate::executor::{PermissionError, VMConfig, VirtualMachine};

    fn compile(code: &str) -> crate::bytecode::BytecodeProgram {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;
        compiler.compile(&program).expect("compile failed")
    }

    #[test]
    fn extern_c_declaration_derives_ffi_permission() {
        // §4.8.3: compiling an `extern C fn` stamps `Ffi` into the extern-stub
        // blob (the blob carrying `CallForeign`); the linker's transitive union
        // surfaces it in `total_required_permissions` (baked into the hash).
        let bytecode = compile(r#"extern C fn labs(x: int) -> int from "c";"#);
        let ca = bytecode
            .content_addressed
            .as_ref()
            .expect("content-addressed program");
        let stub = ca
            .function_store
            .values()
            .find(|b| b.name == "labs")
            .expect("labs stub blob");
        assert!(
            stub.required_permissions
                .contains(&shape_abi_v1::Permission::Ffi),
            "extern C stub blob must require Ffi"
        );
        let linked = crate::linker::link(ca).expect("link");
        assert!(
            linked
                .total_required_permissions
                .contains(&shape_abi_v1::Permission::Ffi),
            "linker transitive union must contain Ffi"
        );
    }

    #[test]
    fn python_declaration_derives_ffi_permission() {
        let bytecode = compile(
            r#"
            fn python add(a: int, b: int) -> Result<int> {
                return a + b
            }
        "#,
        );
        let ca = bytecode
            .content_addressed
            .as_ref()
            .expect("content-addressed program");
        let stub = ca
            .function_store
            .values()
            .find(|b| b.name == "add")
            .expect("add stub blob");
        assert!(
            stub.required_permissions
                .contains(&shape_abi_v1::Permission::Ffi),
            "fn python stub blob must require Ffi"
        );
    }

    #[test]
    fn non_foreign_program_does_not_require_ffi() {
        let bytecode = compile(r#"fn add(a: int, b: int) -> int { return a + b }"#);
        let ca = bytecode.content_addressed.as_ref().expect("ca");
        let linked = crate::linker::link(ca).expect("link");
        assert!(
            !linked
                .total_required_permissions
                .contains(&shape_abi_v1::Permission::Ffi),
            "a program with no foreign code must not require Ffi"
        );
    }

    #[test]
    fn deterministic_context_refuses_foreign_program_at_load() {
        // §4.8.3 (Q6): a Deterministic execution context refuses a foreign-
        // bearing program at LOAD time — even when Ffi itself is granted.
        let bytecode =
            compile(r#"fn python add(a: int, b: int) -> Result<int> { return a + b }"#);
        let ca = bytecode
            .content_addressed
            .clone()
            .expect("content-addressed program");
        let granted = shape_abi_v1::PermissionSet::from([
            shape_abi_v1::Permission::Ffi,
            shape_abi_v1::Permission::Deterministic,
        ]);
        let mut vm = VirtualMachine::new(VMConfig::default());
        let err = vm
            .load_program_with_permissions(ca, &granted)
            .expect_err("deterministic + foreign must refuse at load");
        assert!(
            matches!(err, PermissionError::DeterministicForeignRefused),
            "expected DeterministicForeignRefused, got {err:?}"
        );
    }

    #[test]
    fn foreign_program_loads_when_ffi_granted_without_determinism() {
        let bytecode =
            compile(r#"fn python add(a: int, b: int) -> Result<int> { return a + b }"#);
        let ca = bytecode.content_addressed.clone().expect("ca");
        let granted = shape_abi_v1::PermissionSet::from([shape_abi_v1::Permission::Ffi]);
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program_with_permissions(ca, &granted)
            .expect("Ffi-granted non-deterministic load should succeed");
    }

    #[test]
    fn foreign_program_refused_at_load_without_ffi_grant() {
        let bytecode =
            compile(r#"fn python add(a: int, b: int) -> Result<int> { return a + b }"#);
        let ca = bytecode.content_addressed.clone().expect("ca");
        // readonly does not include Ffi (shape-abi-v1 preset invariant).
        let granted = shape_abi_v1::PermissionSet::readonly();
        let mut vm = VirtualMachine::new(VMConfig::default());
        let err = vm
            .load_program_with_permissions(ca, &granted)
            .expect_err("no-Ffi sandbox must refuse foreign program at load");
        match err {
            PermissionError::InsufficientPermissions { missing, .. } => {
                assert!(
                    missing.contains(&shape_abi_v1::Permission::Ffi),
                    "missing set must name Ffi"
                );
            }
            other => panic!("expected InsufficientPermissions, got {other:?}"),
        }
    }

    #[test]
    fn native_foreign_call_refused_before_dlopen_without_ffi() {
        // §4.8.2 phase 1: the coarse `Ffi` presence check runs BEFORE link-now
        // (path resolution + dlopen). Point a native entry at a NONEXISTENT
        // library: if control reached `link_native_function`/dlopen, the error
        // would be a link failure ("Failed to link ... No such file"). Getting
        // the permission-refusal error INSTEAD proves the check precedes dlopen
        // — the .so's ELF constructors never run.
        let bytecode = compile(
            r#"extern C fn missing_sym(x: int) -> int from "/nonexistent/libshape_ffi_probe_does_not_exist.so";"#,
        );
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        // Handles start unlinked (lazy). Install a granted set WITHOUT Ffi.
        vm.foreign_fn_handles = vec![None];
        vm.set_permissions(
            Some(shape_abi_v1::PermissionSet::from([
                shape_abi_v1::Permission::FsRead,
            ])),
            None,
        );
        let err = vm
            .invoke_foreign_kinded(0, &[])
            .expect_err("foreign call without Ffi must be refused");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("requires permission Ffi"),
            "expected Ffi permission refusal, got: {msg}"
        );
        assert!(
            !msg.contains("Failed to link") && !msg.to_lowercase().contains("no such file"),
            "dlopen must NOT have been attempted (constructor must not run): {msg}"
        );
    }

    #[test]
    fn native_foreign_call_refused_before_dlopen_under_determinism() {
        // Deterministic backstop (§4.8.3): even with a nonexistent library, a
        // Deterministic granted context refuses the call before any link/dlopen.
        let bytecode = compile(
            r#"extern C fn det_missing(x: int) -> int from "/nonexistent/libshape_ffi_det_probe.so";"#,
        );
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.foreign_fn_handles = vec![None];
        vm.set_permissions(
            Some(shape_abi_v1::PermissionSet::from([
                shape_abi_v1::Permission::Ffi,
                shape_abi_v1::Permission::Deterministic,
            ])),
            None,
        );
        let err = vm
            .invoke_foreign_kinded(0, &[])
            .expect_err("deterministic foreign call must be refused");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("deterministic execution cannot run foreign code"),
            "expected deterministic refusal, got: {msg}"
        );
        assert!(
            !msg.contains("Failed to link") && !msg.to_lowercase().contains("no such file"),
            "dlopen must NOT have been attempted: {msg}"
        );
    }

    #[test]
    fn local_run_allows_foreign_call_unscoped() {
        // OQ13 posture: a trusted-local run (granted == None) grants Ffi
        // unscoped, so the permission check passes and control reaches link-now.
        // With a nonexistent library the outcome is a LINK failure — proving the
        // permission gate did NOT block it (it got past the check to dlopen).
        let bytecode = compile(
            r#"extern C fn local_missing(x: int) -> int from "/nonexistent/libshape_ffi_local_probe.so";"#,
        );
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.foreign_fn_handles = vec![None];
        // granted == None (trusted local): Ffi is unscoped.
        let err = vm
            .invoke_foreign_kinded(0, &[])
            .expect_err("nonexistent library must fail to link");
        let msg = format!("{err:?}");
        assert!(
            !msg.contains("requires permission Ffi") && !msg.contains("deterministic execution"),
            "local run must NOT be blocked by the permission gate: {msg}"
        );
        assert!(
            msg.contains("Failed to link"),
            "expected a link failure past the permission gate, got: {msg}"
        );
    }

    // ----------------------------------------------------------------------
    // WF-2F axis C — wire-serve receiver posture (§4.6 / OQ-6, ratified
    // 2026-07-05): `ffi_languages` enforced as a strict OPT-IN allow-list for
    // dynamic foreign languages on the network path, with the deliberate
    // asymmetry against the local unscoped-empty-means-all posture proven
    // above by `local_run_allows_foreign_call_unscoped`.
    // ----------------------------------------------------------------------

    fn python_vm() -> (VirtualMachine, ()) {
        let bytecode =
            compile(r#"fn python padd(a: int, b: int) -> Result<int> { return a + b }"#);
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.foreign_fn_handles = vec![None];
        (vm, ())
    }

    fn scope_with_langs(langs: &[&str]) -> shape_abi_v1::ScopeConstraints {
        let mut sc = shape_abi_v1::ScopeConstraints::none();
        sc.ffi_languages = langs.iter().map(|s| s.to_string()).collect();
        sc
    }

    #[test]
    fn receiver_strict_refuses_dynamic_language_not_opted_in() {
        // Strict receiver + EMPTY ffi_languages: a `fn python` call is refused
        // at the phase-1 gate BEFORE the runtime lookup — the strict-empty
        // posture means "refuse all dynamic foreign" unless opted in.
        let (mut vm, _) = python_vm();
        vm.set_permissions(
            Some(shape_abi_v1::PermissionSet::from([
                shape_abi_v1::Permission::Ffi,
            ])),
            Some(scope_with_langs(&[])),
        );
        vm.ffi_receiver_strict = true;
        let err = vm
            .invoke_foreign_kinded(0, &[])
            .expect_err("strict receiver with empty ffi_languages must refuse python");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("has not opted into the 'python'"),
            "expected strict opt-in refusal naming python, got: {msg}"
        );
        // Proves the refusal PRECEDES the runtime lookup (never-die ordering).
        assert!(
            !msg.contains("no extension provides language"),
            "gate must refuse before the runtime lookup, got: {msg}"
        );
    }

    #[test]
    fn receiver_strict_admits_dynamic_language_when_opted_in() {
        // Strict receiver + ffi_languages = ["python"]: the gate PASSES and
        // control reaches the runtime lookup (which fails here because no
        // python runtime is registered in the test VM — a DIFFERENT error,
        // proving the language opt-in let it through).
        let (mut vm, _) = python_vm();
        vm.set_permissions(
            Some(shape_abi_v1::PermissionSet::from([
                shape_abi_v1::Permission::Ffi,
            ])),
            Some(scope_with_langs(&["python"])),
        );
        vm.ffi_receiver_strict = true;
        let err = vm
            .invoke_foreign_kinded(0, &[])
            .expect_err("no runtime registered → link-now failure past the gate");
        let msg = format!("{err:?}");
        assert!(
            !msg.contains("has not opted into"),
            "opted-in language must NOT be refused by the strict gate: {msg}"
        );
        assert!(
            msg.contains("no extension provides language"),
            "expected a runtime-lookup failure past the gate, got: {msg}"
        );
    }

    #[test]
    fn receiver_strict_does_not_gate_extern_c_by_language() {
        // `extern C` is governed by `Ffi` + `ffi_libraries`/`ffi_symbols`, NOT
        // by `ffi_languages`. A strict receiver with empty ffi_languages must
        // still let a native call through the language gate (it then fails at
        // link because the library does not exist — a link error, not a
        // language refusal).
        let bytecode = compile(
            r#"extern C fn strict_native(x: int) -> int from "/nonexistent/libshape_ffi_strict_probe.so";"#,
        );
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.foreign_fn_handles = vec![None];
        vm.set_permissions(
            Some(shape_abi_v1::PermissionSet::from([
                shape_abi_v1::Permission::Ffi,
            ])),
            Some(scope_with_langs(&[])),
        );
        vm.ffi_receiver_strict = true;
        let err = vm
            .invoke_foreign_kinded(0, &[])
            .expect_err("nonexistent native library must fail to link");
        let msg = format!("{err:?}");
        assert!(
            !msg.contains("opted into") && !msg.contains("ffi_languages"),
            "extern C must NOT be gated by the ffi_languages strict list: {msg}"
        );
        assert!(
            msg.contains("Failed to link"),
            "expected a native link failure past the language gate, got: {msg}"
        );
    }
}

#[cfg(test)]
mod ctype_spelling_tests {
    //! ffi-rebuild §4.10 S2 / OQ4 — canonical lowercase C-view/slice spellings
    //! (`cview`/`cmut`/`cslice`/`cmut_slice`) with the CamelCase forms accepted
    //! as one-release deprecated aliases.
    use crate::compiler::BytecodeCompiler;

    fn native_signature(code: &str, fn_name: &str) -> String {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;
        let bytecode = compiler.compile(&program).expect("compile failed");
        bytecode
            .foreign_functions
            .iter()
            .find(|e| e.name == fn_name)
            .and_then(|e| e.native_abi.as_ref())
            .map(|spec| spec.signature.clone())
            .unwrap_or_else(|| panic!("no native signature for '{fn_name}'"))
    }

    #[test]
    fn lowercase_cview_cmut_compile_and_produce_canonical_signature() {
        let sig = native_signature(
            r#"
            type C QuoteC { bid: f64, ask: f64 }
            extern C fn quote_mid(q: cview<QuoteC>) -> f64 from "libquote";
            extern C fn quote_fill(q: cmut<QuoteC>, v: f64) -> void from "libquote";
            "#,
            "quote_mid",
        );
        assert!(sig.contains("cview<QuoteC>"), "signature: {sig}");
    }

    #[test]
    fn lowercase_cslice_cmut_slice_compile() {
        let sig = native_signature(
            r#"extern C fn bump(xs: cmut_slice<i32>) -> void from "lib";"#,
            "bump",
        );
        assert!(sig.contains("cmut_slice<i32>"), "signature: {sig}");
        let sig = native_signature(
            r#"extern C fn sum(xs: cslice<i32>) -> i32 from "lib";"#,
            "sum",
        );
        assert!(sig.contains("cslice<i32>"), "signature: {sig}");
    }

    #[test]
    fn camelcase_forms_are_aliases_of_the_lowercase_spellings() {
        // CamelCase and lowercase must lower to the identical native C signature
        // — they are one type, one deprecated spelling of the other.
        assert_eq!(
            native_signature(
                r#"type C Q { x: f64 } extern C fn f(q: CView<Q>) -> f64 from "l";"#,
                "f",
            ),
            native_signature(
                r#"type C Q { x: f64 } extern C fn f(q: cview<Q>) -> f64 from "l";"#,
                "f",
            ),
        );
        assert_eq!(
            native_signature(
                r#"extern C fn f(xs: CMutSlice<i32>) -> void from "l";"#,
                "f",
            ),
            native_signature(
                r#"extern C fn f(xs: cmut_slice<i32>) -> void from "l";"#,
                "f",
            ),
        );
    }

    #[test]
    fn deprecated_alias_detection_flags_camelcase_only() {
        use shape_ast::ast::{TypeAnnotation, TypePath};
        let camel = TypeAnnotation::Generic {
            name: TypePath::simple("CView"),
            args: vec![TypeAnnotation::Reference(TypePath::simple("Q"))],
        };
        let lower = TypeAnnotation::Generic {
            name: TypePath::simple("cview"),
            args: vec![TypeAnnotation::Reference(TypePath::simple("Q"))],
        };
        assert_eq!(
            BytecodeCompiler::deprecated_ctype_alias_in(&camel),
            Some(("CView", "cview"))
        );
        assert_eq!(BytecodeCompiler::deprecated_ctype_alias_in(&lower), None);
    }
}
