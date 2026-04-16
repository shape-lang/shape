//! Foreign function (extern C) compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use shape_value::ValueWordExt;
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
                    TypeAnnotation::Basic(type_name) => type_name.clone(),
                    TypeAnnotation::Reference(type_name) => type_name.to_string(),
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
}
