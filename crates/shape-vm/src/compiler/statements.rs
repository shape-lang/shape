//! Statement and item compilation

use crate::bytecode::{Function, Instruction, OpCode, Operand};
use shape_ast::ast::{
    AnnotationTargetKind, DestructurePattern, EnumDef, EnumMemberKind, ExportItem, Expr,
    FunctionDef, FunctionParameter, Item, Literal, ModuleDecl, ObjectEntry, Query, Span, Statement,
    TypeAnnotation, VarKind,
};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_schema::{EnumVariantInfo, FieldType};

use super::{BytecodeCompiler, DropKind, ImportedSymbol, ParamPassMode, StructGenericInfo};

#[derive(Debug, Clone)]
struct NativeFieldLayoutSpec {
    c_type: String,
    size: u64,
    align: u64,
}

impl BytecodeCompiler {
    fn emit_comptime_internal_call(
        &mut self,
        method: &str,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<()> {
        let call = Expr::MethodCall {
            receiver: Box::new(Expr::Identifier("__comptime__".to_string(), span)),
            method: method.to_string(),
            args,
            named_args: Vec::new(),
            span,
        };
        let prev = self.allow_internal_comptime_namespace;
        self.allow_internal_comptime_namespace = true;
        let compile_result = self.compile_expr(&call);
        self.allow_internal_comptime_namespace = prev;
        compile_result?;
        self.emit(Instruction::simple(OpCode::Pop));
        Ok(())
    }

    fn emit_comptime_extend_directive(
        &mut self,
        extend: &shape_ast::ast::ExtendStatement,
        span: Span,
    ) -> Result<()> {
        let payload = serde_json::to_string(extend).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to serialize comptime extend directive: {}", e),
            location: Some(self.span_to_source_location(span)),
        })?;
        self.emit_comptime_internal_call(
            "__emit_extend",
            vec![Expr::Literal(Literal::String(payload), span)],
            span,
        )
    }

    fn emit_comptime_remove_directive(&mut self, span: Span) -> Result<()> {
        self.emit_comptime_internal_call("__emit_remove", Vec::new(), span)
    }

    fn emit_comptime_set_param_value_directive(
        &mut self,
        param_name: &str,
        expression: &Expr,
        span: Span,
    ) -> Result<()> {
        self.emit_comptime_internal_call(
            "__emit_set_param_value",
            vec![
                Expr::Literal(Literal::String(param_name.to_string()), span),
                expression.clone(),
            ],
            span,
        )
    }

    fn emit_comptime_set_param_type_directive(
        &mut self,
        param_name: &str,
        type_annotation: &TypeAnnotation,
        span: Span,
    ) -> Result<()> {
        let payload =
            serde_json::to_string(type_annotation).map_err(|e| ShapeError::RuntimeError {
                message: format!("Failed to serialize comptime param type directive: {}", e),
                location: Some(self.span_to_source_location(span)),
            })?;
        self.emit_comptime_internal_call(
            "__emit_set_param_type",
            vec![
                Expr::Literal(Literal::String(param_name.to_string()), span),
                Expr::Literal(Literal::String(payload), span),
            ],
            span,
        )
    }

    fn emit_comptime_set_return_type_directive(
        &mut self,
        type_annotation: &TypeAnnotation,
        span: Span,
    ) -> Result<()> {
        let payload =
            serde_json::to_string(type_annotation).map_err(|e| ShapeError::RuntimeError {
                message: format!("Failed to serialize comptime return type directive: {}", e),
                location: Some(self.span_to_source_location(span)),
            })?;
        self.emit_comptime_internal_call(
            "__emit_set_return_type",
            vec![Expr::Literal(Literal::String(payload), span)],
            span,
        )
    }

    fn emit_comptime_set_return_expr_directive(
        &mut self,
        expression: &Expr,
        span: Span,
    ) -> Result<()> {
        self.emit_comptime_internal_call("__emit_set_return_type", vec![expression.clone()], span)
    }

    fn emit_comptime_replace_body_directive(
        &mut self,
        body: &[Statement],
        span: Span,
    ) -> Result<()> {
        let payload = serde_json::to_string(body).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to serialize comptime replace-body directive: {}", e),
            location: Some(self.span_to_source_location(span)),
        })?;
        self.emit_comptime_internal_call(
            "__emit_replace_body",
            vec![Expr::Literal(Literal::String(payload), span)],
            span,
        )
    }

    fn emit_comptime_replace_body_expr_directive(
        &mut self,
        expression: &Expr,
        span: Span,
    ) -> Result<()> {
        self.emit_comptime_internal_call("__emit_replace_body", vec![expression.clone()], span)
    }

    fn emit_comptime_replace_module_expr_directive(
        &mut self,
        expression: &Expr,
        span: Span,
    ) -> Result<()> {
        self.emit_comptime_internal_call("__emit_replace_module", vec![expression.clone()], span)
    }

    pub(super) fn register_item_functions(&mut self, item: &Item) -> Result<()> {
        match item {
            Item::Function(func_def, _) => self.register_function(func_def),
            Item::Module(module_def, _) => {
                let module_path = self.current_module_path_for(module_def.name.as_str());
                self.module_scope_stack.push(module_path.clone());
                let register_result = (|| -> Result<()> {
                    for inner in &module_def.items {
                        let qualified = self.qualify_module_item(inner, &module_path)?;
                        self.register_item_functions(&qualified)?;
                    }
                    Ok(())
                })();
                self.module_scope_stack.pop();
                register_result
            }
            Item::Trait(trait_def, _) => {
                self.known_traits.insert(trait_def.name.clone());
                self.trait_defs
                    .insert(trait_def.name.clone(), trait_def.clone());
                Ok(())
            }
            Item::ForeignFunction(def, _) => {
                // Register as a normal function so call sites resolve the name.
                // Caller-visible arity excludes `out` params.
                let caller_visible = def.params.iter().filter(|p| !p.is_out).count();
                self.function_arity_bounds
                    .insert(def.name.clone(), (caller_visible, caller_visible));
                self.function_const_params
                    .insert(def.name.clone(), Vec::new());
                let (ref_params, ref_mutates) = Self::native_param_reference_contract(def);
                let (vis_ref_params, vis_ref_mutates) = if def.params.iter().any(|p| p.is_out) {
                    let mut vrp = Vec::new();
                    let mut vrm = Vec::new();
                    for (i, p) in def.params.iter().enumerate() {
                        if !p.is_out {
                            vrp.push(ref_params.get(i).copied().unwrap_or(false));
                            vrm.push(ref_mutates.get(i).copied().unwrap_or(false));
                        }
                    }
                    (vrp, vrm)
                } else {
                    (ref_params, ref_mutates)
                };

                let func = crate::bytecode::Function {
                    name: def.name.clone(),
                    arity: caller_visible as u16,
                    param_names: def
                        .params
                        .iter()
                        .filter(|p| !p.is_out)
                        .flat_map(|p| p.get_identifiers())
                        .collect(),
                    locals_count: 0,
                    entry_point: 0,
                    body_length: 0,
                    is_closure: false,
                    captures_count: 0,
                    is_async: def.is_async,
                    ref_params: vis_ref_params,
                    ref_mutates: vis_ref_mutates,
                    mutable_captures: Vec::new(),
                    frame_descriptor: None,
                    osr_entry_points: Vec::new(),
                };
                self.program.functions.push(func);

                // Store the foreign function def so call sites can resolve
                // the declared return type (must be Result<T> for dynamic languages).
                self.foreign_function_defs
                    .insert(def.name.clone(), def.clone());

                Ok(())
            }
            Item::Export(export, _) => match &export.item {
                ExportItem::Function(func_def) => self.register_function(func_def),
                ExportItem::Trait(trait_def) => {
                    self.known_traits.insert(trait_def.name.clone());
                    self.trait_defs
                        .insert(trait_def.name.clone(), trait_def.clone());
                    Ok(())
                }
                ExportItem::ForeignFunction(def) => {
                    // Same registration as Item::ForeignFunction
                    let caller_visible = def.params.iter().filter(|p| !p.is_out).count();
                    self.function_arity_bounds
                        .insert(def.name.clone(), (caller_visible, caller_visible));
                    self.function_const_params
                        .insert(def.name.clone(), Vec::new());
                    let (ref_params, ref_mutates) = Self::native_param_reference_contract(def);
                    let (vis_ref_params, vis_ref_mutates) =
                        if def.params.iter().any(|p| p.is_out) {
                            let mut vrp = Vec::new();
                            let mut vrm = Vec::new();
                            for (i, p) in def.params.iter().enumerate() {
                                if !p.is_out {
                                    vrp.push(ref_params.get(i).copied().unwrap_or(false));
                                    vrm.push(ref_mutates.get(i).copied().unwrap_or(false));
                                }
                            }
                            (vrp, vrm)
                        } else {
                            (ref_params, ref_mutates)
                        };

                    let func = crate::bytecode::Function {
                        name: def.name.clone(),
                        arity: caller_visible as u16,
                        param_names: def
                            .params
                            .iter()
                            .filter(|p| !p.is_out)
                            .flat_map(|p| p.get_identifiers())
                            .collect(),
                        locals_count: 0,
                        entry_point: 0,
                        body_length: 0,
                        is_closure: false,
                        captures_count: 0,
                        is_async: def.is_async,
                        ref_params: vis_ref_params,
                        ref_mutates: vis_ref_mutates,
                        mutable_captures: Vec::new(),
                        frame_descriptor: None,
                        osr_entry_points: Vec::new(),
                    };
                    self.program.functions.push(func);

                    self.foreign_function_defs
                        .insert(def.name.clone(), def.clone());

                    Ok(())
                }
                _ => Ok(()),
            },
            Item::Extend(extend, _) => {
                // Desugar extend methods to functions with implicit `self` receiver param.
                for method in &extend.methods {
                    let func_def = self.desugar_extend_method(method, &extend.type_name)?;
                    self.register_function(&func_def)?;
                }
                Ok(())
            }
            Item::Impl(impl_block, _) => {
                // Impl blocks use scoped UFCS names.
                // - default impl: "Type::method" (legacy compatibility)
                // - named impl: "Trait::Type::ImplName::method"
                // This prevents conflicts when multiple named impls exist.
                let trait_name = match &impl_block.trait_name {
                    shape_ast::ast::types::TypeName::Simple(n) => n.as_str(),
                    shape_ast::ast::types::TypeName::Generic { name, .. } => name.as_str(),
                };
                let type_name = match &impl_block.target_type {
                    shape_ast::ast::types::TypeName::Simple(n) => n.as_str(),
                    shape_ast::ast::types::TypeName::Generic { name, .. } => name.as_str(),
                };
                let impl_name = impl_block.impl_name.as_deref();

                // From/TryFrom impls use reverse-conversion desugaring:
                // the method takes an explicit `value` param (no implicit self),
                // and we auto-derive Into/TryInto trait symbols on the source type.
                if trait_name == "From" || trait_name == "TryFrom" {
                    return self.compile_from_impl(impl_block, trait_name, type_name);
                }

                // Collect names of methods explicitly provided in the impl block
                let overridden: std::collections::HashSet<&str> =
                    impl_block.methods.iter().map(|m| m.name.as_str()).collect();

                for method in &impl_block.methods {
                    let func_def = self.desugar_impl_method(
                        method,
                        trait_name,
                        type_name,
                        impl_name,
                        &impl_block.target_type,
                    )?;
                    self.program.register_trait_method_symbol(
                        trait_name,
                        type_name,
                        impl_name,
                        &method.name,
                        &func_def.name,
                    );
                    self.register_function(&func_def)?;

                    // Track drop kind per type (sync, async, or both)
                    if trait_name == "Drop" && method.name == "drop" {
                        let type_key = type_name.to_string();
                        let existing = self.drop_type_info.get(&type_key).copied();
                        let new_kind = if method.is_async {
                            match existing {
                                Some(DropKind::SyncOnly) | Some(DropKind::Both) => DropKind::Both,
                                _ => DropKind::AsyncOnly,
                            }
                        } else {
                            match existing {
                                Some(DropKind::AsyncOnly) | Some(DropKind::Both) => DropKind::Both,
                                _ => DropKind::SyncOnly,
                            }
                        };
                        self.drop_type_info.insert(type_key, new_kind);
                    }
                }

                // Install default methods from the trait definition that were not overridden
                if let Some(trait_def) = self.trait_defs.get(trait_name).cloned() {
                    for member in &trait_def.members {
                        if let shape_ast::ast::types::TraitMember::Default(default_method) = member
                        {
                            if !overridden.contains(default_method.name.as_str()) {
                                let func_def = self.desugar_impl_method(
                                    default_method,
                                    trait_name,
                                    type_name,
                                    impl_name,
                                    &impl_block.target_type,
                                )?;
                                self.program.register_trait_method_symbol(
                                    trait_name,
                                    type_name,
                                    impl_name,
                                    &default_method.name,
                                    &func_def.name,
                                );
                                self.register_function(&func_def)?;
                            }
                        }
                    }
                }

                // BUG-4.6 fix: Register the trait impl in the type inference
                // environment so that `implements()` can see it at comptime.
                let all_method_names: Vec<String> =
                    impl_block.methods.iter().map(|m| m.name.clone()).collect();
                if let Some(selector) = impl_name {
                    let _ = self.type_inference.env.register_trait_impl_named(
                        trait_name,
                        type_name,
                        selector,
                        all_method_names,
                    );
                } else {
                    let _ = self.type_inference.env.register_trait_impl(
                        trait_name,
                        type_name,
                        all_method_names,
                    );
                }

                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Register a function definition
    pub(super) fn register_function(&mut self, func_def: &FunctionDef) -> Result<()> {
        // Detect duplicate function definitions (Shape does not support overloading).
        // Skip names containing "::" (trait impl methods) or "." (extend methods)
        // — those are type-qualified and live in separate namespaces.
        if !func_def.name.contains("::")
            && !func_def.name.contains('.')
        {
            if let Some(existing) = self.program.functions.iter().find(|f| f.name == func_def.name) {
                // Allow idempotent re-registration from module inlining: when the
                // prelude and an explicitly imported module both define the same helper
                // function (e.g., `percentile`), silently keep the first definition
                // if arities match. Different arities indicate a genuine conflict.
                if existing.arity == func_def.params.len() as u16 {
                    return Ok(());
                }
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Duplicate function definition: '{}' is already defined",
                        func_def.name
                    ),
                    location: Some(self.span_to_source_location(func_def.name_span)),
                });
            }
        }

        self.function_defs
            .insert(func_def.name.clone(), func_def.clone());

        let total_params = func_def.params.len();
        let mut required_params = total_params;
        let mut saw_default = false;
        let mut const_params = Vec::new();
        for (idx, param) in func_def.params.iter().enumerate() {
            if param.is_const {
                const_params.push(idx);
            }
            if param.default_value.is_some() {
                if !saw_default {
                    required_params = idx;
                    saw_default = true;
                }
            } else if saw_default {
                return Err(ShapeError::SemanticError {
                    message: "Required parameter cannot follow a parameter with a default value"
                        .to_string(),
                    location: Some(self.span_to_source_location(param.span())),
                });
            }
        }

        self.function_arity_bounds
            .insert(func_def.name.clone(), (required_params, total_params));
        self.function_const_params
            .insert(func_def.name.clone(), const_params);

        let inferred_param_modes = self
            .inferred_param_pass_modes
            .get(&func_def.name)
            .cloned()
            .unwrap_or_default();
        let mut ref_params: Vec<bool> = Vec::with_capacity(func_def.params.len());
        let mut ref_mutates: Vec<bool> = Vec::with_capacity(func_def.params.len());
        for (idx, param) in func_def.params.iter().enumerate() {
            let fallback = if param.is_reference {
                ParamPassMode::ByRefShared
            } else {
                ParamPassMode::ByValue
            };
            let mode = inferred_param_modes.get(idx).copied().unwrap_or(fallback);
            ref_params.push(mode.is_reference());
            ref_mutates.push(mode.is_exclusive());
        }

        let func = Function {
            name: func_def.name.clone(),
            arity: func_def.params.len() as u16,
            param_names: func_def
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect(),
            locals_count: 0, // Will be updated during compilation
            entry_point: 0,  // Will be updated during compilation
            body_length: 0,  // Will be updated during compilation
            is_closure: false,
            captures_count: 0,
            is_async: func_def.is_async,
            ref_params,
            ref_mutates,
            mutable_captures: Vec::new(),
            frame_descriptor: None,
            osr_entry_points: Vec::new(),
        };

        self.program.functions.push(func);

        // Register function return type for typed opcode emission.
        // When a function has an explicit return type annotation (e.g., `: int`),
        // record it so that call sites can propagate NumericType through expressions
        // like `fib(n-1) + fib(n-2)` and emit AddInt instead of generic Add.
        if let Some(ref return_type) = func_def.return_type {
            if let Some(type_name) = return_type.as_simple_name() {
                self.type_tracker
                    .register_function_return_type(&func_def.name, type_name);
            }
        }

        Ok(())
    }

    /// Compile a top-level item with context about whether it's the last item
    /// If is_last is true and the item is an expression, keep the result on the stack
    pub(super) fn compile_item_with_context(&mut self, item: &Item, is_last: bool) -> Result<()> {
        match item {
            Item::Function(func_def, _) => self.compile_function(func_def)?,
            Item::Module(module_def, span) => {
                self.compile_module_decl(module_def, *span)?;
            }
            Item::VariableDecl(var_decl, _) => {
                // ModuleBinding variable — register the variable even if the initializer fails,
                // to prevent cascading "Undefined variable" errors on later references.
                let init_err = if let Some(init_expr) = &var_decl.value {
                    match self.compile_expr(init_expr) {
                        Ok(()) => None,
                        Err(e) => {
                            // Push null as placeholder so the variable still gets registered
                            self.emit(Instruction::simple(OpCode::PushNull));
                            Some(e)
                        }
                    }
                } else {
                    self.emit(Instruction::simple(OpCode::PushNull));
                    None
                };

                if let Some(name) = var_decl.pattern.as_identifier() {
                    let binding_idx = self.get_or_create_module_binding(name);
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));

                    // Propagate type info from annotation or initializer expression
                    if let Some(ref type_ann) = var_decl.type_annotation {
                        if let Some(type_name) =
                            Self::tracked_type_name_from_annotation(type_ann)
                        {
                            self.set_module_binding_type_info(binding_idx, &type_name);
                        }
                    } else {
                        let is_mutable = var_decl.kind == shape_ast::ast::VarKind::Var;
                        self.propagate_initializer_type_to_slot(binding_idx, false, is_mutable);
                    }

                    // Track for auto-drop at program exit
                    let binding_type_name = self
                        .type_tracker
                        .get_binding_type(binding_idx)
                        .and_then(|info| info.type_name.clone());
                    let drop_kind = binding_type_name
                        .as_ref()
                        .and_then(|tn| self.drop_type_info.get(tn).copied())
                        .or_else(|| {
                            var_decl
                                .type_annotation
                                .as_ref()
                                .and_then(|ann| self.annotation_drop_kind(ann))
                        });
                    if drop_kind.is_some() {
                        let is_async = match drop_kind {
                            Some(DropKind::AsyncOnly) => true,
                            Some(DropKind::Both) => false,
                            Some(DropKind::SyncOnly) | None => false,
                        };
                        self.track_drop_module_binding(binding_idx, is_async);
                    }
                } else {
                    self.compile_destructure_pattern_global(&var_decl.pattern)?;
                }

                if let Some(e) = init_err {
                    return Err(e);
                }
            }
            Item::Assignment(assign, _) => {
                self.compile_statement(&Statement::Assignment(assign.clone(), Span::DUMMY))?;
            }
            Item::Expression(expr, _) => {
                self.compile_expr(expr)?;
                // Only pop if not the last item - keep last expression result on stack
                if !is_last {
                    self.emit(Instruction::simple(OpCode::Pop));
                }
            }
            Item::Statement(stmt, _) => {
                // For expression statements that are the last item, keep result on stack
                if is_last {
                    if let Statement::Expression(expr, _) = stmt {
                        self.compile_expr(expr)?;
                        // Don't emit Pop - keep result on stack
                        return Ok(());
                    }
                }
                self.compile_statement(stmt)?;
            }
            Item::Export(export, export_span) => {
                // If the export has a source variable declaration (pub let/const/var),
                // compile it so the initialization is actually executed.
                if let Some(ref var_decl) = export.source_decl {
                    if let Some(init_expr) = &var_decl.value {
                        self.compile_expr(init_expr)?;
                    } else {
                        self.emit(Instruction::simple(OpCode::PushNull));
                    }
                    if let Some(name) = var_decl.pattern.as_identifier() {
                        let binding_idx = self.get_or_create_module_binding(name);
                        self.emit(Instruction::new(
                            OpCode::StoreModuleBinding,
                            Some(Operand::ModuleBinding(binding_idx)),
                        ));
                    }
                }
                match &export.item {
                    ExportItem::Function(func_def) => self.compile_function(func_def)?,
                    ExportItem::Enum(enum_def) => self.register_enum(enum_def)?,
                    ExportItem::Struct(struct_def) => {
                        self.register_struct_type(struct_def, *export_span)?;
                        if self.struct_types.contains_key(&struct_def.name) {
                            self.emit_annotation_lifecycle_calls_for_type(
                                &struct_def.name,
                                &struct_def.annotations,
                            )?;
                        }
                    }
                    ExportItem::Interface(_) => {} // no-op for now
                    ExportItem::Trait(_) => {} // no-op for now (trait registration happens in type system)
                    ExportItem::ForeignFunction(def) => self.compile_foreign_function(def)?,
                    _ => {}
                }
            }
            Item::Stream(_stream, _) => {
                return Err(ShapeError::StreamError {
                    message: "Streaming functionality has been removed".to_string(),
                    stream_name: None,
                });
            }
            Item::TypeAlias(type_alias, _) => {
                // Track type alias for meta validation
                let base_type_name = match &type_alias.type_annotation {
                    TypeAnnotation::Reference(name) | TypeAnnotation::Basic(name) => {
                        Some(name.clone())
                    }
                    _ => None,
                };
                self.type_aliases.insert(
                    type_alias.name.clone(),
                    base_type_name
                        .clone()
                        .unwrap_or_else(|| format!("{:?}", type_alias.type_annotation)),
                );

                // Apply comptime field overrides from type alias
                // e.g., type EUR = Currency { symbol: "€" } overrides Currency's comptime symbol
                if let (Some(base_name), Some(overrides)) =
                    (&base_type_name, &type_alias.meta_param_overrides)
                {
                    use shape_ast::ast::Literal;
                    use shape_value::ValueWord;
                    use std::sync::Arc;

                    // Start with base type's comptime fields (if any)
                    let mut alias_comptime = self
                        .comptime_fields
                        .get(base_name)
                        .cloned()
                        .unwrap_or_default();

                    for (field_name, expr) in overrides {
                        let value = match expr {
                            Expr::Literal(Literal::Number(n), _) => ValueWord::from_f64(*n),
                            Expr::Literal(Literal::Int(n), _) => ValueWord::from_f64(*n as f64),
                            Expr::Literal(Literal::String(s), _) => {
                                ValueWord::from_string(Arc::new(s.clone()))
                            }
                            Expr::Literal(Literal::Bool(b), _) => ValueWord::from_bool(*b),
                            Expr::Literal(Literal::None, _) => ValueWord::none(),
                            _ => {
                                return Err(ShapeError::SemanticError {
                                    message: format!(
                                        "Comptime field override '{}' on type alias '{}' must be a literal",
                                        field_name, type_alias.name
                                    ),
                                    location: None,
                                });
                            }
                        };
                        alias_comptime.insert(field_name.clone(), value);
                    }

                    if !alias_comptime.is_empty() {
                        self.comptime_fields
                            .insert(type_alias.name.clone(), alias_comptime);
                    }
                }
            }
            Item::StructType(struct_def, span) => {
                self.register_struct_type(struct_def, *span)?;
                if self.struct_types.contains_key(&struct_def.name) {
                    self.emit_annotation_lifecycle_calls_for_type(
                        &struct_def.name,
                        &struct_def.annotations,
                    )?;
                }
            }
            Item::Enum(enum_def, _) => {
                self.register_enum(enum_def)?;
            }
            // Meta/Format definitions removed — formatting now uses Display trait
            Item::Import(import_stmt, _) => {
                // Import handling is now done by executor pre-resolution
                // via the unified runtime module loader.
                // Imported module AST items are inlined via prelude injection
                // before compilation (single-pass, no index remapping).
                //
                // At self point in compile_item, imports should already have been
                // processed by pre-resolution. If we reach here, the import
                // is either:
                // 1. Being compiled standalone (no module context) - skip for now
                // 2. A future extension point for runtime imports
                //
                // For now, we register the imported names as known functions
                // that can be resolved later.
                self.register_import_names(import_stmt)?;
            }
            Item::Extend(extend, _) => {
                // Compile desugared extend methods
                for method in &extend.methods {
                    let func_def = self.desugar_extend_method(method, &extend.type_name)?;
                    self.compile_function(&func_def)?;
                }
            }
            Item::Impl(impl_block, _) => {
                // Compile impl block methods with scoped names
                let trait_name = match &impl_block.trait_name {
                    shape_ast::ast::types::TypeName::Simple(n) => n.as_str(),
                    shape_ast::ast::types::TypeName::Generic { name, .. } => name.as_str(),
                };
                let type_name = match &impl_block.target_type {
                    shape_ast::ast::types::TypeName::Simple(n) => n.as_str(),
                    shape_ast::ast::types::TypeName::Generic { name, .. } => name.as_str(),
                };
                let impl_name = impl_block.impl_name.as_deref();

                // From/TryFrom: compile the from/tryFrom method + synthetic wrapper
                if trait_name == "From" || trait_name == "TryFrom" {
                    return self.compile_from_impl_bodies(impl_block, trait_name, type_name);
                }

                // Collect names of methods explicitly provided in the impl block
                let overridden: std::collections::HashSet<&str> =
                    impl_block.methods.iter().map(|m| m.name.as_str()).collect();

                for method in &impl_block.methods {
                    let func_def = self.desugar_impl_method(
                        method,
                        trait_name,
                        type_name,
                        impl_name,
                        &impl_block.target_type,
                    )?;
                    self.compile_function(&func_def)?;
                }

                // Compile default methods from the trait definition that were not overridden
                if let Some(trait_def) = self.trait_defs.get(trait_name).cloned() {
                    for member in &trait_def.members {
                        if let shape_ast::ast::types::TraitMember::Default(default_method) = member
                        {
                            if !overridden.contains(default_method.name.as_str()) {
                                let func_def = self.desugar_impl_method(
                                    default_method,
                                    trait_name,
                                    type_name,
                                    impl_name,
                                    &impl_block.target_type,
                                )?;
                                self.compile_function(&func_def)?;
                            }
                        }
                    }
                }
            }
            Item::AnnotationDef(ann_def, _) => {
                self.compile_annotation_def(ann_def)?;
            }
            Item::Comptime(stmts, span) => {
                // Execute comptime block at compile time (side-effects only; result discarded)
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
                let comptime_helpers = self.collect_comptime_helpers();
                let execution = super::comptime::execute_comptime(
                    stmts,
                    &comptime_helpers,
                    &extensions,
                    trait_impls,
                    known_type_symbols,
                )
                .map_err(|e| ShapeError::RuntimeError {
                    message: format!(
                        "Comptime block evaluation failed: {}",
                        super::helpers::strip_error_prefix(&e)
                    ),
                    location: Some(self.span_to_source_location(*span)),
                })?;
                self.process_comptime_directives(execution.directives, "")
                    .map_err(|e| ShapeError::RuntimeError {
                        message: format!("Comptime block directive processing failed: {}", e),
                        location: Some(self.span_to_source_location(*span)),
                    })?;
            }
            Item::Query(query, _span) => {
                self.compile_query(query)?;
                // Pop the query result unless self is the last item
                if !is_last {
                    self.emit(Instruction::simple(OpCode::Pop));
                }
            }
            Item::ForeignFunction(def, _) => self.compile_foreign_function(def)?,
            _ => {} // Skip other items for now
        }
        Ok(())
    }

    /// Register imported names for symbol resolution
    ///
    /// This allows the compiler to recognize imported functions when
    /// they are called later in the code.
    fn register_import_names(&mut self, import_stmt: &shape_ast::ast::ImportStmt) -> Result<()> {
        use shape_ast::ast::ImportItems;

        // Check permissions before registering imports.
        // Clone to avoid borrow conflict with &mut self in check_import_permissions.
        if let Some(pset) = self.permission_set.clone() {
            self.check_import_permissions(import_stmt, &pset)?;
        }

        match &import_stmt.items {
            ImportItems::Named(specs) => {
                for spec in specs {
                    let local_name = spec.alias.as_ref().unwrap_or(&spec.name);
                    // Register as a known import - actual function resolution
                    // happens when the imported module's bytecode is merged
                    self.imported_names.insert(
                        local_name.clone(),
                        ImportedSymbol {
                            original_name: spec.name.clone(),
                            module_path: import_stmt.from.clone(),
                        },
                    );
                }
            }
            ImportItems::Namespace { name, alias } => {
                // `use module.path` or `use module.path as alias`
                // Register the local namespace binding as a module_binding.
                let local_name = alias.as_ref().unwrap_or(name);
                let binding_idx = self.get_or_create_module_binding(local_name);
                self.module_namespace_bindings.insert(local_name.clone());
                let module_path = if import_stmt.from.is_empty() {
                    name.as_str()
                } else {
                    import_stmt.from.as_str()
                };
                // Predeclare module object schema so runtime can instantiate
                // module module_bindings without synthesizing schemas dynamically.
                self.register_extension_module_schema(module_path);
                let module_schema_name = format!("__mod_{}", module_path);
                if self
                    .type_tracker
                    .schema_registry()
                    .get(&module_schema_name)
                    .is_some()
                {
                    self.set_module_binding_type_info(binding_idx, &module_schema_name);
                }
                // The module object will be provided at runtime by the VM
                let _ = binding_idx;
            }
        }
        Ok(())
    }

    /// Check whether the imported symbols are allowed by the active permission set.
    ///
    /// For named imports (`from "file" import { read_text }`), checks each function
    /// individually. For namespace imports (`use http`), checks the whole module.
    fn check_import_permissions(
        &mut self,
        import_stmt: &shape_ast::ast::ImportStmt,
        pset: &shape_abi_v1::PermissionSet,
    ) -> Result<()> {
        use shape_ast::ast::ImportItems;
        use shape_runtime::stdlib::capability_tags;

        // Extract the module name from the import path.
        // Paths like "std::file", "file", "std/file" all resolve to "file".
        let module_name = Self::extract_module_name(&import_stmt.from);

        match &import_stmt.items {
            ImportItems::Named(specs) => {
                for spec in specs {
                    let required = capability_tags::required_permissions(module_name, &spec.name);
                    if !required.is_empty() && !required.is_subset(pset) {
                        let missing = required.difference(pset);
                        let missing_names: Vec<&str> = missing.iter().map(|p| p.name()).collect();
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "Permission denied: {module_name}::{} requires {} capability, \
                                 but the active permission set does not include it. \
                                 Add the permission to [permissions] in shape.toml or use a less \
                                 restrictive preset.",
                                spec.name,
                                missing_names.join(", "),
                            ),
                            location: None,
                        });
                    }
                    self.record_blob_permissions(module_name, &spec.name);
                }
            }
            ImportItems::Namespace { .. } => {
                // For namespace imports, check the entire module's permission envelope.
                // If the module requires any permissions not granted, deny the import.
                let required = capability_tags::module_permissions(module_name);
                if !required.is_empty() && !required.is_subset(pset) {
                    let missing = required.difference(pset);
                    let missing_names: Vec<&str> = missing.iter().map(|p| p.name()).collect();
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Permission denied: module '{module_name}' requires {} capabilities, \
                             but the active permission set does not include them. \
                             Add the permissions to [permissions] in shape.toml or use a less \
                             restrictive preset.",
                            missing_names.join(", "),
                        ),
                        location: None,
                    });
                }
                // Record module-level permissions for namespace imports in the current blob
                if let Some(ref mut blob) = self.current_blob_builder {
                    let module_perms = capability_tags::module_permissions(module_name);
                    blob.record_permissions(&module_perms);
                }
            }
        }
        Ok(())
    }

    /// Extract the leaf module name from an import path.
    ///
    /// `"std::file"` → `"file"`, `"file"` → `"file"`, `"std/io"` → `"io"`
    fn extract_module_name(path: &str) -> &str {
        path.rsplit(|c| c == ':' || c == '/')
            .find(|s| !s.is_empty())
            .unwrap_or(path)
    }

    pub(super) fn register_extension_module_schema(&mut self, module_path: &str) {
        let Some(registry) = self.extension_registry.as_ref() else {
            return;
        };
        let Some(module) = registry.iter().rev().find(|m| m.name == module_path) else {
            return;
        };

        for schema in &module.type_schemas {
            if self
                .type_tracker
                .schema_registry()
                .get(&schema.name)
                .is_none()
            {
                self.type_tracker
                    .schema_registry_mut()
                    .register(schema.clone());
            }
        }

        let schema_name = format!("__mod_{}", module_path);
        if self
            .type_tracker
            .schema_registry()
            .get(&schema_name)
            .is_some()
        {
            return;
        }

        let mut export_names: Vec<String> = module
            .export_names_available(self.comptime_mode)
            .into_iter()
            .map(|name| name.to_string())
            .collect();

        for artifact in &module.module_artifacts {
            if artifact.module_path != module_path {
                continue;
            }
            let Some(source) = artifact.source.as_deref() else {
                continue;
            };
            if let Ok(names) =
                shape_runtime::module_loader::collect_exported_function_names_from_source(
                    &artifact.module_path,
                    source,
                )
            {
                export_names.extend(names);
            }
        }

        export_names.sort();
        export_names.dedup();

        let fields: Vec<(String, FieldType)> = export_names
            .into_iter()
            .map(|name| (name, FieldType::Any))
            .collect();
        self.type_tracker
            .schema_registry_mut()
            .register_type(schema_name, fields);
    }

    /// Register an enum definition in the TypeSchemaRegistry
    fn register_enum(&mut self, enum_def: &EnumDef) -> Result<()> {
        let variants: Vec<EnumVariantInfo> = enum_def
            .members
            .iter()
            .enumerate()
            .map(|(id, member)| {
                let payload_fields = match &member.kind {
                    EnumMemberKind::Unit { .. } => 0,
                    EnumMemberKind::Tuple(types) => types.len() as u16,
                    EnumMemberKind::Struct(fields) => fields.len() as u16,
                };
                EnumVariantInfo::new(&member.name, id as u16, payload_fields)
            })
            .collect();

        let schema = shape_runtime::type_schema::TypeSchema::new_enum(&enum_def.name, variants);
        self.type_tracker.schema_registry_mut().register(schema);
        Ok(())
    }

    /// Pre-register items from an imported module (enums, struct types, functions).
    ///
    /// Called by the LSP before compilation to make imported enums/types known
    /// to the compiler's type tracker. Reuses `register_enum` as single source of truth.
    pub fn register_imported_items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Export(export, _) => {
                    match &export.item {
                        ExportItem::Enum(enum_def) => {
                            let _ = self.register_enum(enum_def);
                        }
                        ExportItem::Struct(struct_def) => {
                            // Register struct type fields so the compiler knows about them
                            let _ = self.register_struct_type(struct_def, Span::DUMMY);
                        }
                        ExportItem::Function(func_def) => {
                            // Register function so it's known during compilation
                            let _ = self.register_function(func_def);
                        }
                        _ => {}
                    }
                }
                Item::Enum(enum_def, _) => {
                    let _ = self.register_enum(enum_def);
                }
                _ => {}
            }
        }
    }

    /// Register a meta definition in the format registry
    ///
    // Meta compilation methods removed — formatting now uses Display trait

    /// Desugar an extend method to a FunctionDef with implicit `self` first param.
    ///
    /// `extend Number { method double() { self * 2 } }`
    /// becomes: `function double(self) { self * 2 }`
    ///
    /// UFCS handles the rest: `(5).double()` → `double(5)` → self = 5
    pub(super) fn desugar_extend_method(
        &self,
        method: &shape_ast::ast::types::MethodDef,
        target_type: &shape_ast::ast::TypeName,
    ) -> Result<FunctionDef> {
        let receiver_type = Some(Self::type_name_to_annotation(target_type));
        let (params, body) = self.desugar_method_signature_and_body(method, receiver_type)?;

        // Extend methods use qualified "Type.method" names to avoid collisions
        // with free functions (e.g., prelude's `sum` vs extend Point { method sum() }).
        let type_str = match target_type {
            shape_ast::ast::TypeName::Simple(n) => n.clone(),
            shape_ast::ast::TypeName::Generic { name, .. } => name.clone(),
        };

        Ok(FunctionDef {
            name: format!("{}.{}", type_str, method.name),
            name_span: Span::DUMMY,
            declaring_module_path: method.declaring_module_path.clone(),
            doc_comment: None,
            params,
            return_type: method.return_type.clone(),
            body,
            type_params: Some(Vec::new()),
            annotations: method.annotations.clone(),
            is_async: method.is_async,
            is_comptime: false,
            where_clause: None,
        })
    }

    /// Desugar an impl method to a scoped FunctionDef.
    ///
    /// - Default impl:
    ///   `impl Queryable for DbTable { method filter(pred) { ... } }`
    ///   becomes: `function DbTable::filter(self, pred) { ... }`
    /// - Named impl:
    ///   `impl Display for User as JsonDisplay { method display() { ... } }`
    ///   becomes: `function Display::User::JsonDisplay::display(self) { ... }`
    ///
    /// Named impls use trait/type/impl prefixes to avoid collisions.
    fn desugar_impl_method(
        &self,
        method: &shape_ast::ast::types::MethodDef,
        trait_name: &str,
        type_name: &str,
        impl_name: Option<&str>,
        target_type: &shape_ast::ast::TypeName,
    ) -> Result<FunctionDef> {
        let receiver_type = Some(Self::type_name_to_annotation(target_type));
        let (params, body) = self.desugar_method_signature_and_body(method, receiver_type)?;

        // Async drop methods are named "drop_async" so both sync and async
        // variants can coexist in the function name index.
        let method_name = if trait_name == "Drop" && method.name == "drop" && method.is_async {
            "drop_async".to_string()
        } else {
            method.name.clone()
        };
        let fn_name = if let Some(name) = impl_name {
            format!("{}::{}::{}::{}", trait_name, type_name, name, method_name)
        } else {
            format!("{}::{}", type_name, method_name)
        };

        Ok(FunctionDef {
            name: fn_name,
            name_span: Span::DUMMY,
            declaring_module_path: method.declaring_module_path.clone(),
            doc_comment: None,
            params,
            return_type: method.return_type.clone(),
            body,
            type_params: Some(Vec::new()),
            annotations: method.annotations.clone(),
            is_async: method.is_async,
            is_comptime: false,
            where_clause: None,
        })
    }

    /// Build desugared method params/body with implicit receiver handling.
    ///
    /// Canonical receiver is `self`.
    fn desugar_method_signature_and_body(
        &self,
        method: &shape_ast::ast::types::MethodDef,
        receiver_type: Option<shape_ast::ast::TypeAnnotation>,
    ) -> Result<(Vec<FunctionParameter>, Vec<Statement>)> {
        if let Some(receiver) = method
            .params
            .first()
            .and_then(|p| p.pattern.as_identifier())
        {
            if receiver == "self" {
                let location = method
                    .params
                    .first()
                    .map(|p| self.span_to_source_location(p.span()));
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Method '{}' has an explicit `self` parameter, but method receivers are implicit. Use `method {}(...)` without `self`.",
                        method.name, method.name
                    ),
                    location,
                });
            }
        }

        let mut params = vec![FunctionParameter {
            pattern: shape_ast::ast::DestructurePattern::Identifier(
                "self".to_string(),
                Span::DUMMY,
            ),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: receiver_type,
            default_value: None,
        }];
        params.extend(method.params.clone());

        Ok((params, method.body.clone()))
    }

    /// Compile a `From` or `TryFrom` impl block.
    ///
    /// Unlike normal impl methods (which inject implicit `self`), From/TryFrom
    /// methods are constructors: `from(value: Source) -> Target`. The value
    /// parameter sits at local slot 0 with no receiver.
    ///
    /// Auto-derives:
    /// - `impl From<S> for T`  → `Into<T>::into` on S (direct alias)
    ///                          + `TryInto<T>::tryInto` on S (wrapper → Ok())
    /// - `impl TryFrom<S> for T` → `TryInto<T>::tryInto` on S (direct alias)
    fn compile_from_impl(
        &mut self,
        impl_block: &shape_ast::ast::types::ImplBlock,
        trait_name: &str,
        target_type: &str,
    ) -> Result<()> {
        // Extract source type from generic args: From<Source> → Source
        let source_type = match &impl_block.trait_name {
            shape_ast::ast::types::TypeName::Generic { type_args, .. } if !type_args.is_empty() => {
                match &type_args[0] {
                    TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => name.clone(),
                    other => {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "{} impl requires a simple source type, found {:?}",
                                trait_name, other
                            ),
                            location: None,
                        });
                    }
                }
            }
            _ => {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "{} impl requires a generic type argument, e.g., {}<string>",
                        trait_name, trait_name
                    ),
                    location: None,
                });
            }
        };

        // Named impl selector defaults to the target type name so that
        // `as TargetType` / `as TargetType?` dispatch finds the right symbol.
        let selector = impl_block.impl_name.as_deref().unwrap_or(target_type);

        for method in &impl_block.methods {
            let func_def =
                self.desugar_from_method(method, trait_name, target_type, &source_type)?;
            let from_fn_name = func_def.name.clone();

            // Register From/TryFrom trait method symbol on the target type
            self.program.register_trait_method_symbol(
                trait_name,
                target_type,
                Some(&source_type),
                &method.name,
                &from_fn_name,
            );
            self.register_function(&func_def)?;

            // Auto-derive Into/TryInto on the source type
            if trait_name == "From" {
                // From<S> for T → Into<T>::into on S = direct alias (same fn)
                self.program.register_trait_method_symbol(
                    "Into",
                    &source_type,
                    Some(selector),
                    "into",
                    &from_fn_name,
                );

                // From<S> for T → TryInto<T>::tryInto on S = wrapper (from + Ok)
                let wrapper_name =
                    self.emit_from_to_tryinto_wrapper(&from_fn_name, &source_type, target_type)?;
                self.program.register_trait_method_symbol(
                    "TryInto",
                    &source_type,
                    Some(selector),
                    "tryInto",
                    &wrapper_name,
                );

                // Register trait impls in type inference environment
                let _ = self.type_inference.env.register_trait_impl_named(
                    "Into",
                    &source_type,
                    selector,
                    vec!["into".to_string()],
                );
                let _ = self.type_inference.env.register_trait_impl_named(
                    "TryInto",
                    &source_type,
                    selector,
                    vec!["tryInto".to_string()],
                );
            } else {
                // TryFrom<S> for T → TryInto<T>::tryInto on S = direct alias
                self.program.register_trait_method_symbol(
                    "TryInto",
                    &source_type,
                    Some(selector),
                    "tryInto",
                    &from_fn_name,
                );

                // Register TryInto trait impl in type inference environment
                let _ = self.type_inference.env.register_trait_impl_named(
                    "TryInto",
                    &source_type,
                    selector,
                    vec!["tryInto".to_string()],
                );
            }
        }

        // Register From/TryFrom trait impl on target type
        let all_method_names: Vec<String> =
            impl_block.methods.iter().map(|m| m.name.clone()).collect();
        let _ = self.type_inference.env.register_trait_impl_named(
            trait_name,
            target_type,
            &source_type,
            all_method_names,
        );

        Ok(())
    }

    /// Compile From/TryFrom impl method bodies (and the synthetic TryInto wrapper).
    ///
    /// Called from `compile_item_with_context` — the registration pass already
    /// happened in `compile_from_impl` / `register_item_functions`.
    fn compile_from_impl_bodies(
        &mut self,
        impl_block: &shape_ast::ast::types::ImplBlock,
        trait_name: &str,
        target_type: &str,
    ) -> Result<()> {
        let source_type = match &impl_block.trait_name {
            shape_ast::ast::types::TypeName::Generic { type_args, .. } if !type_args.is_empty() => {
                match &type_args[0] {
                    TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => name.clone(),
                    _ => return Ok(()), // error already reported in registration
                }
            }
            _ => return Ok(()),
        };

        for method in &impl_block.methods {
            let func_def =
                self.desugar_from_method(method, trait_name, target_type, &source_type)?;
            self.compile_function(&func_def)?;
        }

        // Also compile the synthetic TryInto wrapper for From impls
        if trait_name == "From" {
            for method in &impl_block.methods {
                let from_fn_name = format!(
                    "{}::{}::{}::{}",
                    trait_name, target_type, source_type, method.name
                );
                let wrapper_name = format!("__from_tryinto_{}_{}", source_type, target_type);
                // The wrapper was already registered; now compile its body
                if let Some(func_def) = self.function_defs.get(&wrapper_name).cloned() {
                    let _ = self.compile_function(&func_def);
                    // Suppress errors: if Ok() or the from fn is not yet available, it
                    // will be resolved at link time.
                    let _ = from_fn_name; // used above in the format
                }
            }
        }

        Ok(())
    }

    /// Desugar a From/TryFrom method WITHOUT implicit self injection.
    ///
    /// `From::from(value: S)` is a constructor — `value` sits at local slot 0.
    /// Function name: `"From::TargetType::SourceType::method_name"`
    fn desugar_from_method(
        &self,
        method: &shape_ast::ast::types::MethodDef,
        trait_name: &str,
        target_type: &str,
        source_type: &str,
    ) -> Result<FunctionDef> {
        // Verify no explicit `self` parameter
        if let Some(first) = method
            .params
            .first()
            .and_then(|p| p.pattern.as_identifier())
        {
            if first == "self" {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "{}::{} methods are constructors and must not have a `self` parameter",
                        trait_name, method.name
                    ),
                    location: None,
                });
            }
        }

        let fn_name = format!(
            "{}::{}::{}::{}",
            trait_name, target_type, source_type, method.name
        );

        Ok(FunctionDef {
            name: fn_name,
            name_span: Span::DUMMY,
            declaring_module_path: method.declaring_module_path.clone(),
            doc_comment: None,
            params: method.params.clone(),
            return_type: method.return_type.clone(),
            body: method.body.clone(),
            type_params: Some(Vec::new()),
            annotations: Vec::new(),
            is_async: method.is_async,
            is_comptime: false,
            where_clause: None,
        })
    }

    /// Emit a synthetic wrapper function that calls a From::from function
    /// and wraps its result in Ok() for TryInto compatibility.
    ///
    /// Generated function: `__from_tryinto_{source}_{target}(value) -> Ok(from(value))`
    fn emit_from_to_tryinto_wrapper(
        &mut self,
        from_fn_name: &str,
        source_type: &str,
        target_type: &str,
    ) -> Result<String> {
        let wrapper_name = format!("__from_tryinto_{}_{}", source_type, target_type);

        // Create a synthetic FunctionDef whose body calls from() and wraps in Ok()
        let span = Span::DUMMY;
        let body = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "Ok".to_string(),
                args: vec![Expr::FunctionCall {
                    name: from_fn_name.to_string(),
                    args: vec![Expr::Identifier("value".to_string(), span)],
                    named_args: Vec::new(),
                    span,
                }],
                named_args: Vec::new(),
                span,
            }),
            span,
        )];

        let func_def = FunctionDef {
            name: wrapper_name.clone(),
            name_span: span,
            declaring_module_path: None,
            doc_comment: None,
            params: vec![FunctionParameter {
                pattern: DestructurePattern::Identifier("value".to_string(), span),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: None,
                default_value: None,
            }],
            return_type: None,
            body,
            type_params: Some(Vec::new()),
            annotations: Vec::new(),
            is_async: false,
            is_comptime: false,
            where_clause: None,
        };

        self.register_function(&func_def)?;

        Ok(wrapper_name)
    }

    fn type_name_to_annotation(
        type_name: &shape_ast::ast::TypeName,
    ) -> shape_ast::ast::TypeAnnotation {
        match type_name {
            shape_ast::ast::TypeName::Simple(name) => {
                shape_ast::ast::TypeAnnotation::Basic(name.clone())
            }
            shape_ast::ast::TypeName::Generic { name, type_args } => {
                shape_ast::ast::TypeAnnotation::Generic {
                    name: name.clone(),
                    args: type_args.clone(),
                }
            }
        }
    }

    /// Compile an annotation definition.
    ///
    /// Each handler is compiled as an internal function:
    /// - before(args, ctx) → `{name}___before(self, period, args, ctx)`
    /// - after(args, result, ctx) → `{name}___after(self, period, args, result, ctx)`
    ///
    /// `self` is the annotated item (function/method/property).
    /// Annotation params (e.g., `period`) are prepended after `self`.
    fn compile_annotation_def(&mut self, ann_def: &shape_ast::ast::AnnotationDef) -> Result<()> {
        use crate::bytecode::CompiledAnnotation;
        use shape_ast::ast::AnnotationHandlerType;

        let mut compiled = CompiledAnnotation {
            name: ann_def.name.clone(),
            param_names: ann_def
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect(),
            before_handler: None,
            after_handler: None,
            on_define_handler: None,
            metadata_handler: None,
            comptime_pre_handler: None,
            comptime_post_handler: None,
            allowed_targets: Vec::new(),
        };

        for handler in &ann_def.handlers {
            // Comptime handlers are stored as AST (not compiled to bytecode).
            // They are executed at compile time when the annotation is applied.
            match handler.handler_type {
                AnnotationHandlerType::ComptimePre => {
                    compiled.comptime_pre_handler = Some(handler.clone());
                    continue;
                }
                AnnotationHandlerType::ComptimePost => {
                    compiled.comptime_post_handler = Some(handler.clone());
                    continue;
                }
                _ => {}
            }

            if handler.params.iter().any(|p| p.is_variadic) {
                return Err(ShapeError::SemanticError {
                    message:
                        "Variadic annotation handler params (`...args`) are only supported on comptime handlers"
                            .to_string(),
                    location: Some(self.span_to_source_location(handler.span)),
                });
            }

            let handler_type_str = match handler.handler_type {
                AnnotationHandlerType::Before => "before",
                AnnotationHandlerType::After => "after",
                AnnotationHandlerType::OnDefine => "on_define",
                AnnotationHandlerType::Metadata => "metadata",
                AnnotationHandlerType::ComptimePre => unreachable!(),
                AnnotationHandlerType::ComptimePost => unreachable!(),
            };

            let func_name = format!("{}___{}", ann_def.name, handler_type_str);

            // Build function params: self + annotation_params + handler_params
            let mut params = vec![FunctionParameter {
                pattern: shape_ast::ast::DestructurePattern::Identifier(
                    "self".to_string(),
                    Span::DUMMY,
                ),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: None,
                default_value: None,
            }];
            // Add annotation params (e.g., period)
            for ann_param in &ann_def.params {
                params.push(ann_param.clone());
            }
            // Add handler params (e.g., args, ctx)
            for param in &handler.params {
                let inferred_type = if param.name == "ctx" {
                    Some(TypeAnnotation::Object(vec![
                        shape_ast::ast::ObjectTypeField {
                            name: "state".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("unknown".to_string()),
                            annotations: vec![],
                        },
                        shape_ast::ast::ObjectTypeField {
                            name: "event_log".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("unknown".to_string()))),
                            annotations: vec![],
                        },
                    ]))
                } else if matches!(
                    handler.handler_type,
                    AnnotationHandlerType::OnDefine | AnnotationHandlerType::Metadata
                ) && (param.name == "fn" || param.name == "target")
                {
                    Some(TypeAnnotation::Object(vec![
                        shape_ast::ast::ObjectTypeField {
                            name: "name".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("string".to_string()),
                            annotations: vec![],
                        },
                        shape_ast::ast::ObjectTypeField {
                            name: "kind".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("string".to_string()),
                            annotations: vec![],
                        },
                        shape_ast::ast::ObjectTypeField {
                            name: "id".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("int".to_string()),
                            annotations: vec![],
                        },
                    ]))
                } else {
                    None
                };

                params.push(FunctionParameter {
                    pattern: shape_ast::ast::DestructurePattern::Identifier(
                        param.name.clone(),
                        Span::DUMMY,
                    ),
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
                    is_out: false,
                    type_annotation: inferred_type,
                    default_value: None,
                });
            }

            // Convert handler body (Expr) to function body (Vec<Statement>)
            let body = vec![Statement::Return(Some(handler.body.clone()), Span::DUMMY)];

            let func_def = FunctionDef {
                name: func_name,
                name_span: Span::DUMMY,
                declaring_module_path: None,
                doc_comment: None,
                params,
                return_type: handler.return_type.clone(),
                body,
                type_params: Some(Vec::new()),
                annotations: Vec::new(),
                is_async: false,
                is_comptime: false,
                where_clause: None,
            };

            self.register_function(&func_def)?;
            self.compile_function(&func_def)?;

            let func_id = (self.program.functions.len() - 1) as u16;

            match handler.handler_type {
                AnnotationHandlerType::Before => compiled.before_handler = Some(func_id),
                AnnotationHandlerType::After => compiled.after_handler = Some(func_id),
                AnnotationHandlerType::OnDefine => compiled.on_define_handler = Some(func_id),
                AnnotationHandlerType::Metadata => compiled.metadata_handler = Some(func_id),
                AnnotationHandlerType::ComptimePre => {} // handled above
                AnnotationHandlerType::ComptimePost => {} // handled above
            }
        }

        // Resolve allowed target kinds.
        // Explicit `targets: [...]` in the annotation definition has priority.
        // Otherwise infer from handlers:
        // before/after handlers only make sense on functions (they wrap calls),
        // lifecycle handlers (on_define/metadata) are definition-time only.
        if let Some(explicit) = &ann_def.allowed_targets {
            compiled.allowed_targets = explicit.clone();
        } else if compiled.before_handler.is_some()
            || compiled.after_handler.is_some()
            || compiled.comptime_pre_handler.is_some()
            || compiled.comptime_post_handler.is_some()
        {
            compiled.allowed_targets =
                vec![shape_ast::ast::functions::AnnotationTargetKind::Function];
        } else if compiled.on_define_handler.is_some() || compiled.metadata_handler.is_some() {
            compiled.allowed_targets = vec![
                shape_ast::ast::functions::AnnotationTargetKind::Function,
                shape_ast::ast::functions::AnnotationTargetKind::Type,
                shape_ast::ast::functions::AnnotationTargetKind::Module,
            ];
        }

        // Enforce that definition-time lifecycle hooks only target definition
        // sites (`function` / `type`).
        if compiled.on_define_handler.is_some() || compiled.metadata_handler.is_some() {
            if compiled.allowed_targets.is_empty() {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Annotation '{}' uses `on_define`/`metadata` and cannot have unrestricted targets. Allowed targets are: function, type, module",
                        ann_def.name
                    ),
                    location: Some(self.span_to_source_location(ann_def.span)),
                });
            }
            if let Some(invalid) = compiled
                .allowed_targets
                .iter()
                .find(|kind| !Self::is_definition_annotation_target(**kind))
            {
                let invalid_label = format!("{:?}", invalid).to_lowercase();
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Annotation '{}' uses `on_define`/`metadata`, but target '{}' is not a definition target. Allowed targets are: function, type, module",
                        ann_def.name, invalid_label
                    ),
                    location: Some(self.span_to_source_location(ann_def.span)),
                });
            }
        }

        self.program
            .compiled_annotations
            .insert(ann_def.name.clone(), compiled);
        Ok(())
    }

    /// Register a struct type definition.
    ///
    /// Comptime fields are baked at compile time and excluded from the runtime TypeSchema.
    /// Their values are stored in `self.comptime_fields` for constant-folded access.
    fn register_struct_type(
        &mut self,
        struct_def: &shape_ast::ast::StructTypeDef,
        span: shape_ast::ast::Span,
    ) -> Result<()> {
        use shape_ast::ast::Literal;
        use shape_runtime::type_schema::{FieldAnnotation, TypeSchemaBuilder};

        // Validate annotation target kinds before type registration.
        for ann in &struct_def.annotations {
            self.validate_annotation_target_usage(
                ann,
                shape_ast::ast::functions::AnnotationTargetKind::Type,
                span,
            )?;
        }

        if struct_def.native_layout.is_some() {
            self.native_layout_types.insert(struct_def.name.clone());
        } else {
            self.native_layout_types.remove(&struct_def.name);
        }

        // Pre-register runtime field layout so comptime-generated methods on
        // `extend target { ... }` can resolve `self.field` statically.
        // If the target is later removed by comptime directives, these
        // placeholders are rolled back below.
        let runtime_field_names: Vec<String> = struct_def
            .fields
            .iter()
            .filter(|f| !f.is_comptime)
            .map(|f| f.name.clone())
            .collect();
        let runtime_field_types = struct_def
            .fields
            .iter()
            .filter(|f| !f.is_comptime)
            .map(|f| (f.name.clone(), f.type_annotation.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        self.struct_types
            .insert(struct_def.name.clone(), (runtime_field_names, span));
        self.struct_generic_info.insert(
            struct_def.name.clone(),
            StructGenericInfo {
                type_params: struct_def.type_params.clone().unwrap_or_default(),
                runtime_field_types,
            },
        );
        if self
            .type_tracker
            .schema_registry()
            .get(&struct_def.name)
            .is_none()
        {
            let runtime_fields: Vec<(String, shape_runtime::type_schema::FieldType)> = struct_def
                .fields
                .iter()
                .filter(|f| !f.is_comptime)
                .map(|f| {
                    (
                        f.name.clone(),
                        Self::type_annotation_to_field_type(&f.type_annotation),
                    )
                })
                .collect();
            self.type_tracker
                .schema_registry_mut()
                .register_type(struct_def.name.clone(), runtime_fields);
        }

        // Execute comptime annotation handlers before registration so
        // `remove target` can suppress type emission entirely.
        if self.execute_struct_comptime_handlers(struct_def)? {
            self.struct_types.remove(&struct_def.name);
            self.struct_generic_info.remove(&struct_def.name);
            return Ok(());
        }

        if struct_def.native_layout.is_some() {
            self.register_native_struct_layout(struct_def, span)?;
        }

        // Build TypeSchema for runtime fields only
        if self
            .type_tracker
            .schema_registry()
            .get(&struct_def.name)
            .is_none()
        {
            let mut builder = TypeSchemaBuilder::new(struct_def.name.clone());
            for field in &struct_def.fields {
                if field.is_comptime {
                    continue;
                }
                let field_type = Self::type_annotation_to_field_type(&field.type_annotation);
                let mut annotations = Vec::new();
                for ann in &field.annotations {
                    let args: Vec<String> = ann
                        .args
                        .iter()
                        .filter_map(Self::eval_annotation_arg)
                        .collect();
                    annotations.push(FieldAnnotation {
                        name: ann.name.clone(),
                        args,
                    });
                }
                builder = builder.field_with_meta(field.name.clone(), field_type, annotations);
            }
            builder.register(self.type_tracker.schema_registry_mut());
        }

        // Bake comptime field values
        let mut comptime_values = std::collections::HashMap::new();
        for field in &struct_def.fields {
            if !field.is_comptime {
                continue;
            }
            if let Some(ref default_expr) = field.default_value {
                let value = match default_expr {
                    Expr::Literal(Literal::Number(n), _) => shape_value::ValueWord::from_f64(*n),
                    Expr::Literal(Literal::Int(n), _) => {
                        shape_value::ValueWord::from_f64(*n as f64)
                    }
                    Expr::Literal(Literal::String(s), _) => {
                        shape_value::ValueWord::from_string(std::sync::Arc::new(s.clone()))
                    }
                    Expr::Literal(Literal::Bool(b), _) => shape_value::ValueWord::from_bool(*b),
                    Expr::Literal(Literal::None, _) => shape_value::ValueWord::none(),
                    _ => {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "Comptime field '{}' on type '{}' must have a literal default value",
                                field.name, struct_def.name
                            ),
                            location: None,
                        });
                    }
                };
                comptime_values.insert(field.name.clone(), value);
            }
            // Comptime fields without a default are allowed — they must be
            // provided via type alias overrides (e.g., type EUR = Currency { symbol: "€" })
        }

        if !comptime_values.is_empty() {
            self.comptime_fields
                .insert(struct_def.name.clone(), comptime_values);
        }

        self.maybe_generate_native_type_conversions(&struct_def.name, span)?;

        Ok(())
    }

    fn register_native_struct_layout(
        &mut self,
        struct_def: &shape_ast::ast::StructTypeDef,
        span: shape_ast::ast::Span,
    ) -> Result<()> {
        if struct_def.type_params.is_some() {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "type C '{}' cannot be generic in this version",
                    struct_def.name
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        if struct_def.fields.iter().any(|f| f.is_comptime) {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "type C '{}' cannot contain comptime fields",
                    struct_def.name
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        let abi = struct_def
            .native_layout
            .as_ref()
            .map(|b| b.abi.clone())
            .unwrap_or_else(|| "C".to_string());
        if abi != "C" {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "type '{}' uses unsupported native ABI '{}'; only C is supported",
                    struct_def.name, abi
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        let mut struct_align: u64 = 1;
        let mut offset: u64 = 0;
        let mut field_layouts = Vec::with_capacity(struct_def.fields.len());

        for field in &struct_def.fields {
            let field_spec =
                self.native_field_layout_spec(&field.type_annotation, span, &struct_def.name)?;
            struct_align = struct_align.max(field_spec.align);
            offset = Self::align_to(offset, field_spec.align);
            if offset > u32::MAX as u64
                || field_spec.size > u32::MAX as u64
                || field_spec.align > u32::MAX as u64
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "type C '{}' layout exceeds supported size/alignment limits",
                        struct_def.name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
            field_layouts.push(crate::bytecode::NativeStructFieldLayout {
                name: field.name.clone(),
                c_type: field_spec.c_type,
                offset: offset as u32,
                size: field_spec.size as u32,
                align: field_spec.align as u32,
            });
            offset = offset.saturating_add(field_spec.size);
        }

        let size = Self::align_to(offset, struct_align);
        if size > u32::MAX as u64 || struct_align > u32::MAX as u64 {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "type C '{}' layout exceeds supported size/alignment limits",
                    struct_def.name
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        let entry = crate::bytecode::NativeStructLayoutEntry {
            name: struct_def.name.clone(),
            abi,
            size: size as u32,
            align: struct_align as u32,
            fields: field_layouts,
        };

        if let Some(existing) = self
            .program
            .native_struct_layouts
            .iter_mut()
            .find(|existing| existing.name == entry.name)
        {
            *existing = entry;
        } else {
            self.program.native_struct_layouts.push(entry);
        }

        Ok(())
    }

    fn align_to(value: u64, align: u64) -> u64 {
        debug_assert!(align > 0);
        let mask = align - 1;
        (value + mask) & !mask
    }

    fn native_field_layout_spec(
        &self,
        ann: &shape_ast::ast::TypeAnnotation,
        span: shape_ast::ast::Span,
        struct_name: &str,
    ) -> Result<NativeFieldLayoutSpec> {
        use shape_ast::ast::TypeAnnotation;

        let pointer = std::mem::size_of::<usize>() as u64;

        let fail = || -> Result<NativeFieldLayoutSpec> {
            Err(ShapeError::SemanticError {
                message: format!(
                    "unsupported type C field type '{}' in '{}'",
                    ann.to_type_string(),
                    struct_name
                ),
                location: Some(self.span_to_source_location(span)),
            })
        };

        match ann {
            TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => {
                if let Some(existing) = self
                    .program
                    .native_struct_layouts
                    .iter()
                    .find(|layout| &layout.name == name)
                {
                    return Ok(NativeFieldLayoutSpec {
                        c_type: name.clone(),
                        size: existing.size as u64,
                        align: existing.align as u64,
                    });
                }

                let spec = match name.as_str() {
                    "f64" | "number" | "Number" | "float" => ("f64", 8, 8),
                    "f32" => ("f32", 4, 4),
                    "i64" | "int" | "integer" | "Int" | "Integer" => ("i64", 8, 8),
                    "i32" => ("i32", 4, 4),
                    "i16" => ("i16", 2, 2),
                    "i8" | "char" => ("i8", 1, 1),
                    "u64" => ("u64", 8, 8),
                    "u32" => ("u32", 4, 4),
                    "u16" => ("u16", 2, 2),
                    "u8" | "byte" => ("u8", 1, 1),
                    "bool" | "boolean" => ("bool", 1, 1),
                    "isize" => ("isize", pointer, pointer),
                    "usize" | "ptr" | "pointer" => ("ptr", pointer, pointer),
                    "string" | "str" | "cstring" => ("cstring", pointer, pointer),
                    _ => return fail(),
                };
                Ok(NativeFieldLayoutSpec {
                    c_type: spec.0.to_string(),
                    size: spec.1,
                    align: spec.2,
                })
            }
            TypeAnnotation::Generic { name, args }
                if name == "Option" && args.len() == 1 =>
            {
                let inner = self.native_field_layout_spec(&args[0], span, struct_name)?;
                if inner.c_type == "cstring" {
                    Ok(NativeFieldLayoutSpec {
                        c_type: "cstring?".to_string(),
                        size: pointer,
                        align: pointer,
                    })
                } else {
                    fail()
                }
            }
            _ => fail(),
        }
    }

    fn maybe_generate_native_type_conversions(
        &mut self,
        type_name: &str,
        span: shape_ast::ast::Span,
    ) -> Result<()> {
        let pair = if self.native_layout_types.contains(type_name) {
            let Some(object_type) = Self::object_type_name_for_native_layout(type_name) else {
                return Ok(());
            };
            if !self.struct_types.contains_key(&object_type)
                || self.native_layout_types.contains(&object_type)
            {
                return Ok(());
            }
            (type_name.to_string(), object_type)
        } else {
            let candidates: Vec<String> = Self::native_layout_name_candidates_for_object(type_name)
                .into_iter()
                .filter(|candidate| self.native_layout_types.contains(candidate))
                .collect();
            if candidates.is_empty() {
                return Ok(());
            }
            if candidates.len() > 1 {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "type '{}' matches multiple `type C` companions ({}) - use one canonical name",
                        type_name,
                        candidates.join(", ")
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
            (candidates[0].clone(), type_name.to_string())
        };

        let pair_key = format!("{}::{}", pair.0, pair.1);
        if self.generated_native_conversion_pairs.contains(&pair_key) {
            return Ok(());
        }

        self.validate_native_conversion_pair(&pair.0, &pair.1, span)?;
        self.generate_native_conversion_direction(&pair.0, &pair.1, span)?;
        self.generate_native_conversion_direction(&pair.1, &pair.0, span)?;
        self.generated_native_conversion_pairs.insert(pair_key);
        Ok(())
    }

    fn object_type_name_for_native_layout(name: &str) -> Option<String> {
        if let Some(base) = name.strip_suffix("Layout")
            && !base.is_empty()
        {
            return Some(base.to_string());
        }
        if let Some(base) = name.strip_suffix('C')
            && !base.is_empty()
        {
            return Some(base.to_string());
        }
        if let Some(base) = name.strip_prefix('C')
            && !base.is_empty()
            && base
                .chars()
                .next()
                .map(|ch| ch.is_ascii_uppercase())
                .unwrap_or(false)
        {
            return Some(base.to_string());
        }
        None
    }

    fn native_layout_name_candidates_for_object(name: &str) -> Vec<String> {
        vec![
            format!("{}Layout", name),
            format!("{}C", name),
            format!("C{}", name),
        ]
    }

    fn validate_native_conversion_pair(
        &self,
        c_type: &str,
        object_type: &str,
        span: shape_ast::ast::Span,
    ) -> Result<()> {
        if !self.native_layout_types.contains(c_type) {
            return Err(ShapeError::SemanticError {
                message: format!("'{}' is not declared as `type C`", c_type),
                location: Some(self.span_to_source_location(span)),
            });
        }
        if self.native_layout_types.contains(object_type) {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "auto conversion target '{}' cannot also be declared as `type C`",
                    object_type
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        let c_type_info =
            self.struct_generic_info
                .get(c_type)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!("missing compiler metadata for `type C {}`", c_type),
                    location: Some(self.span_to_source_location(span)),
                })?;
        let object_type_info =
            self.struct_generic_info
                .get(object_type)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "missing compiler metadata for companion type '{}'",
                        object_type
                    ),
                    location: Some(self.span_to_source_location(span)),
                })?;

        if !c_type_info.type_params.is_empty() || !object_type_info.type_params.is_empty() {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "auto `type C` conversions currently require non-generic types (`{}` <-> `{}`)",
                    c_type, object_type
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        let c_fields = self
            .struct_types
            .get(c_type)
            .map(|(fields, _)| fields)
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!("missing field metadata for `type C {}`", c_type),
                location: Some(self.span_to_source_location(span)),
            })?;
        let object_fields = self
            .struct_types
            .get(object_type)
            .map(|(fields, _)| fields)
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!(
                    "missing field metadata for companion type '{}'",
                    object_type
                ),
                location: Some(self.span_to_source_location(span)),
            })?;

        let c_field_set: std::collections::HashSet<&str> =
            c_fields.iter().map(String::as_str).collect();
        let object_field_set: std::collections::HashSet<&str> =
            object_fields.iter().map(String::as_str).collect();
        if c_field_set != object_field_set {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "auto conversion pair '{}' <-> '{}' must have identical runtime fields",
                    c_type, object_type
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        for field_name in c_field_set {
            let c_ann = c_type_info
                .runtime_field_types
                .get(field_name)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "missing type metadata for field '{}.{}'",
                        c_type, field_name
                    ),
                    location: Some(self.span_to_source_location(span)),
                })?;
            let object_ann = object_type_info
                .runtime_field_types
                .get(field_name)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "missing type metadata for field '{}.{}'",
                        object_type, field_name
                    ),
                    location: Some(self.span_to_source_location(span)),
                })?;
            if c_ann != object_ann {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "field type mismatch for auto conversion '{}.{}' (`{}`) vs '{}.{}' (`{}`)",
                        c_type,
                        field_name,
                        c_ann.to_type_string(),
                        object_type,
                        field_name,
                        object_ann.to_type_string()
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
        }

        Ok(())
    }

    fn generate_native_conversion_direction(
        &mut self,
        source_type: &str,
        target_type: &str,
        span: shape_ast::ast::Span,
    ) -> Result<()> {
        let fn_name = format!(
            "__auto_native_from_{}_to_{}",
            Self::sanitize_auto_symbol(source_type),
            Self::sanitize_auto_symbol(target_type)
        );
        if self.function_defs.contains_key(&fn_name) {
            return Ok(());
        }

        let target_fields = self
            .struct_types
            .get(target_type)
            .map(|(fields, _)| fields.clone())
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!(
                    "missing target type metadata for auto conversion '{}'",
                    target_type
                ),
                location: Some(self.span_to_source_location(span)),
            })?;

        let source_expr = Expr::Identifier("value".to_string(), span);
        let struct_fields = target_fields
            .iter()
            .map(|field| {
                (
                    field.clone(),
                    Expr::PropertyAccess {
                        object: Box::new(source_expr.clone()),
                        property: field.clone(),
                        optional: false,
                        span,
                    },
                )
            })
            .collect::<Vec<_>>();
        let body = vec![Statement::Return(
            Some(Expr::StructLiteral {
                type_name: target_type.to_string(),
                fields: struct_fields,
                span,
            }),
            span,
        )];
        let fn_def = FunctionDef {
            name: fn_name.clone(),
            name_span: span,
            declaring_module_path: None,
            doc_comment: None,
            params: vec![FunctionParameter {
                pattern: DestructurePattern::Identifier("value".to_string(), span),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: Some(TypeAnnotation::Reference(source_type.to_string())),
                default_value: None,
            }],
            return_type: Some(TypeAnnotation::Reference(target_type.to_string())),
            body,
            type_params: Some(Vec::new()),
            annotations: Vec::new(),
            is_async: false,
            is_comptime: false,
            where_clause: None,
        };
        self.register_function(&fn_def)?;
        self.compile_function(&fn_def)?;

        self.program.register_trait_method_symbol(
            "From",
            target_type,
            Some(source_type),
            "from",
            &fn_name,
        );
        self.program.register_trait_method_symbol(
            "Into",
            source_type,
            Some(target_type),
            "into",
            &fn_name,
        );
        let _ = self.type_inference.env.register_trait_impl_named(
            "From",
            target_type,
            source_type,
            vec!["from".to_string()],
        );
        let _ = self.type_inference.env.register_trait_impl_named(
            "Into",
            source_type,
            target_type,
            vec!["into".to_string()],
        );
        Ok(())
    }

    fn sanitize_auto_symbol(name: &str) -> String {
        let mut out = String::with_capacity(name.len());
        for ch in name.chars() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch);
            } else {
                out.push('_');
            }
        }
        out
    }

    /// Execute comptime annotation handlers for a struct type definition.
    ///
    /// Mirrors `execute_comptime_handlers` in functions.rs but uses
    /// `ComptimeTarget::from_type()` to build the target from struct fields.
    fn execute_struct_comptime_handlers(
        &mut self,
        struct_def: &shape_ast::ast::StructTypeDef,
    ) -> Result<bool> {
        let mut removed = false;
        for ann in &struct_def.annotations {
            let compiled = self.program.compiled_annotations.get(&ann.name).cloned();
            if let Some(compiled) = compiled {
                let handlers = [
                    compiled.comptime_pre_handler,
                    compiled.comptime_post_handler,
                ];
                for handler in handlers.into_iter().flatten() {
                    // Build field info for ComptimeTarget::from_type()
                    // Include per-field annotations so comptime handlers can inspect them.
                    let fields: Vec<(
                        String,
                        Option<shape_ast::ast::TypeAnnotation>,
                        Vec<shape_ast::ast::functions::Annotation>,
                    )> = struct_def
                        .fields
                        .iter()
                        .map(|f| {
                            (
                                f.name.clone(),
                                Some(f.type_annotation.clone()),
                                f.annotations.clone(),
                            )
                        })
                        .collect();

                    let target = super::comptime_target::ComptimeTarget::from_type(
                        &struct_def.name,
                        &fields,
                    );
                    let target_value = target.to_nanboxed();
                    let target_name = struct_def.name.clone();
                    let handler_span = handler.span;
                    let execution =
                        self.execute_comptime_annotation_handler(ann, &handler, target_value, &compiled.param_names, &[])?;

                    if self
                        .process_comptime_directives(execution.directives, &target_name)
                        .map_err(|e| ShapeError::RuntimeError {
                            message: format!(
                                "Comptime handler '{}' directive processing failed: {}",
                                ann.name, e
                            ),
                            location: Some(self.span_to_source_location(handler_span)),
                        })?
                    {
                        removed = true;
                        break;
                    }
                }
            }
            if removed {
                break;
            }
        }
        Ok(removed)
    }

    fn current_module_path_for(&self, module_name: &str) -> String {
        if let Some(parent) = self.module_scope_stack.last() {
            format!("{}::{}", parent, module_name)
        } else {
            module_name.to_string()
        }
    }

    fn qualify_module_symbol(module_path: &str, name: &str) -> String {
        format!("{}::{}", module_path, name)
    }

    fn qualify_module_item(&self, item: &Item, module_path: &str) -> Result<Item> {
        match item {
            Item::Function(func, span) => {
                let mut qualified = func.clone();
                qualified.name = Self::qualify_module_symbol(module_path, &func.name);
                Ok(Item::Function(qualified, *span))
            }
            Item::VariableDecl(decl, span) => {
                if decl.kind != VarKind::Const {
                    return Err(ShapeError::SemanticError {
                        message: "module-level variable declarations currently require `const`"
                            .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                let mut qualified = decl.clone();
                let Some(name) = decl.pattern.as_identifier() else {
                    return Err(ShapeError::SemanticError {
                        message:
                            "module-level constants currently require a simple identifier binding"
                                .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                };
                qualified.pattern = DestructurePattern::Identifier(
                    Self::qualify_module_symbol(module_path, name),
                    *span,
                );
                Ok(Item::VariableDecl(qualified, *span))
            }
            Item::Statement(Statement::VariableDecl(decl, stmt_span), item_span) => {
                if decl.kind != VarKind::Const {
                    return Err(ShapeError::SemanticError {
                        message: "module-level variable declarations currently require `const`"
                            .to_string(),
                        location: Some(self.span_to_source_location(*stmt_span)),
                    });
                }
                let mut qualified = decl.clone();
                let Some(name) = decl.pattern.as_identifier() else {
                    return Err(ShapeError::SemanticError {
                        message:
                            "module-level constants currently require a simple identifier binding"
                                .to_string(),
                        location: Some(self.span_to_source_location(*stmt_span)),
                    });
                };
                qualified.pattern = DestructurePattern::Identifier(
                    Self::qualify_module_symbol(module_path, name),
                    *stmt_span,
                );
                Ok(Item::Statement(
                    Statement::VariableDecl(qualified, *stmt_span),
                    *item_span,
                ))
            }
            Item::Statement(Statement::Assignment(assign, stmt_span), item_span) => {
                let mut qualified = assign.clone();
                if let Some(name) = assign.pattern.as_identifier() {
                    qualified.pattern = DestructurePattern::Identifier(
                        Self::qualify_module_symbol(module_path, name),
                        *stmt_span,
                    );
                }
                Ok(Item::Statement(
                    Statement::Assignment(qualified, *stmt_span),
                    *item_span,
                ))
            }
            Item::Export(export, span) if export.source_decl.is_some() => {
                // pub const/let/var: unwrap the source_decl and qualify it as a VariableDecl
                let decl = export.source_decl.as_ref().unwrap();
                if decl.kind != VarKind::Const {
                    return Err(ShapeError::SemanticError {
                        message: "module-level variable declarations currently require `const`"
                            .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                let mut qualified = decl.clone();
                let Some(name) = decl.pattern.as_identifier() else {
                    return Err(ShapeError::SemanticError {
                        message:
                            "module-level constants currently require a simple identifier binding"
                                .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                };
                qualified.pattern = DestructurePattern::Identifier(
                    Self::qualify_module_symbol(module_path, name),
                    *span,
                );
                Ok(Item::VariableDecl(qualified, *span))
            }
            _ => Ok(item.clone()),
        }
    }

    fn collect_module_runtime_exports(
        &self,
        items: &[Item],
        module_path: &str,
    ) -> Vec<(String, String)> {
        let mut exports = Vec::new();
        for item in items {
            match item {
                Item::Function(func, _) => {
                    exports.push((
                        func.name.clone(),
                        Self::qualify_module_symbol(module_path, &func.name),
                    ));
                }
                Item::VariableDecl(decl, _) => {
                    if decl.kind == VarKind::Const
                        && let Some(name) = decl.pattern.as_identifier()
                    {
                        exports.push((
                            name.to_string(),
                            Self::qualify_module_symbol(module_path, name),
                        ));
                    }
                }
                Item::Statement(Statement::VariableDecl(decl, _), _) => {
                    if decl.kind == VarKind::Const
                        && let Some(name) = decl.pattern.as_identifier()
                    {
                        exports.push((
                            name.to_string(),
                            Self::qualify_module_symbol(module_path, name),
                        ));
                    }
                }
                Item::Export(export, _) => {
                    if let Some(ref decl) = export.source_decl {
                        if let Some(name) = decl.pattern.as_identifier() {
                            exports.push((
                                name.to_string(),
                                Self::qualify_module_symbol(module_path, name),
                            ));
                        }
                    }
                }
                Item::Module(module, _) => {
                    exports.push((
                        module.name.clone(),
                        Self::qualify_module_symbol(module_path, &module.name),
                    ));
                }
                _ => {}
            }
        }
        exports.sort_by(|a, b| a.0.cmp(&b.0));
        exports.dedup_by(|a, b| a.0 == b.0);
        exports
    }

    fn module_target_fields(items: &[Item]) -> Vec<(String, String)> {
        let mut fields = Vec::new();
        for item in items {
            match item {
                Item::Function(func, _) => fields.push((func.name.clone(), "function".to_string())),
                Item::VariableDecl(decl, _) => {
                    if let Some(name) = decl.pattern.as_identifier() {
                        let type_name = decl
                            .type_annotation
                            .as_ref()
                            .and_then(TypeAnnotation::as_simple_name)
                            .unwrap_or("any")
                            .to_string();
                        fields.push((name.to_string(), type_name));
                    }
                }
                Item::Statement(Statement::VariableDecl(decl, _), _) => {
                    if let Some(name) = decl.pattern.as_identifier() {
                        let type_name = decl
                            .type_annotation
                            .as_ref()
                            .and_then(TypeAnnotation::as_simple_name)
                            .unwrap_or("any")
                            .to_string();
                        fields.push((name.to_string(), type_name));
                    }
                }
                Item::Export(export, _) => {
                    if let Some(ref decl) = export.source_decl {
                        if let Some(name) = decl.pattern.as_identifier() {
                            let type_name = decl
                                .type_annotation
                                .as_ref()
                                .and_then(TypeAnnotation::as_simple_name)
                                .unwrap_or("any")
                                .to_string();
                            fields.push((name.to_string(), type_name));
                        }
                    }
                }
                Item::StructType(def, _) => fields.push((def.name.clone(), "type".to_string())),
                Item::Enum(def, _) => fields.push((def.name.clone(), "type".to_string())),
                Item::TypeAlias(def, _) => fields.push((def.name.clone(), "type".to_string())),
                Item::Module(def, _) => fields.push((def.name.clone(), "module".to_string())),
                _ => {}
            }
        }
        fields
    }

    fn process_comptime_directives_for_module(
        &mut self,
        directives: Vec<super::comptime_builtins::ComptimeDirective>,
        module_name: &str,
        module_items: &mut Vec<Item>,
    ) -> std::result::Result<bool, String> {
        let mut removed = false;
        for directive in directives {
            match directive {
                super::comptime_builtins::ComptimeDirective::Extend(extend) => {
                    self.apply_comptime_extend(extend, module_name)
                        .map_err(|e| e.to_string())?;
                }
                super::comptime_builtins::ComptimeDirective::RemoveTarget => {
                    removed = true;
                    break;
                }
                super::comptime_builtins::ComptimeDirective::ReplaceModule { items } => {
                    *module_items = items;
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
            }
        }
        Ok(removed)
    }

    fn execute_module_comptime_handlers(
        &mut self,
        module_def: &ModuleDecl,
        module_path: &str,
        module_items: &mut Vec<Item>,
    ) -> Result<bool> {
        let mut removed = false;
        for ann in &module_def.annotations {
            let compiled = self.program.compiled_annotations.get(&ann.name).cloned();
            if let Some(compiled) = compiled {
                let handlers = [
                    compiled.comptime_pre_handler,
                    compiled.comptime_post_handler,
                ];
                for handler in handlers.into_iter().flatten() {
                    let target = super::comptime_target::ComptimeTarget::from_module(
                        module_path,
                        &Self::module_target_fields(module_items),
                    );
                    let target_value = target.to_nanboxed();
                    let handler_span = handler.span;
                    let execution =
                        self.execute_comptime_annotation_handler(ann, &handler, target_value, &compiled.param_names, &[])?;
                    if self
                        .process_comptime_directives_for_module(
                            execution.directives,
                            module_path,
                            module_items,
                        )
                        .map_err(|e| ShapeError::RuntimeError {
                            message: format!(
                                "Comptime handler '{}' directive processing failed: {}",
                                ann.name, e
                            ),
                            location: Some(self.span_to_source_location(handler_span)),
                        })?
                    {
                        removed = true;
                        break;
                    }
                }
            }
            if removed {
                break;
            }
        }
        Ok(removed)
    }

    fn inject_module_local_comptime_helper_aliases(
        &self,
        module_path: &str,
        helpers: &mut Vec<FunctionDef>,
    ) {
        let module_prefix = format!("{}::", module_path);
        let mut seen: std::collections::HashSet<String> =
            helpers.iter().map(|h| h.name.clone()).collect();
        let mut aliases = Vec::new();

        for helper in helpers.iter() {
            let Some(local_name) = helper.name.strip_prefix(&module_prefix) else {
                continue;
            };
            if local_name.contains("::") || !seen.insert(local_name.to_string()) {
                continue;
            }
            let mut alias = helper.clone();
            alias.name = local_name.to_string();
            aliases.push(alias);
        }

        helpers.extend(aliases);
    }

    fn execute_module_inline_comptime_blocks(
        &mut self,
        module_path: &str,
        module_items: &mut Vec<Item>,
    ) -> Result<bool> {
        loop {
            let Some(idx) = module_items
                .iter()
                .position(|item| matches!(item, Item::Comptime(_, _)))
            else {
                break;
            };

            let (stmts, span) = match module_items[idx].clone() {
                Item::Comptime(stmts, span) => (stmts, span),
                _ => unreachable!("index is guarded by position() matcher"),
            };

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
            self.inject_module_local_comptime_helper_aliases(module_path, &mut comptime_helpers);

            let execution = super::comptime::execute_comptime(
                &stmts,
                &comptime_helpers,
                &extensions,
                trait_impls,
                known_type_symbols,
            )
            .map_err(|e| ShapeError::RuntimeError {
                message: format!(
                    "Comptime block evaluation failed: {}",
                    super::helpers::strip_error_prefix(&e)
                ),
                location: Some(self.span_to_source_location(span)),
            })?;

            if self
                .process_comptime_directives_for_module(
                    execution.directives,
                    module_path,
                    module_items,
                )
                .map_err(|e| ShapeError::RuntimeError {
                    message: format!("Comptime block directive processing failed: {}", e),
                    location: Some(self.span_to_source_location(span)),
                })?
            {
                return Ok(true);
            }

            if idx < module_items.len() && matches!(module_items[idx], Item::Comptime(_, _)) {
                module_items.remove(idx);
            }
        }

        Ok(false)
    }

    fn register_missing_module_functions(&mut self, item: &Item) -> Result<()> {
        match item {
            Item::Function(func, _) => {
                if !self.function_defs.contains_key(&func.name) {
                    self.register_function(func)?;
                }
                Ok(())
            }
            Item::Export(export, _) => match &export.item {
                ExportItem::Function(func) => {
                    if !self.function_defs.contains_key(&func.name) {
                        self.register_function(func)?;
                    }
                    Ok(())
                }
                _ => Ok(()),
            },
            Item::Module(module, _) => {
                let module_path = self.current_module_path_for(module.name.as_str());
                self.module_scope_stack.push(module_path.clone());
                let register_result = (|| -> Result<()> {
                    for inner in &module.items {
                        let qualified = self.qualify_module_item(inner, &module_path)?;
                        self.register_missing_module_functions(&qualified)?;
                    }
                    Ok(())
                })();
                self.module_scope_stack.pop();
                register_result
            }
            _ => Ok(()),
        }
    }

    fn compile_module_decl(&mut self, module_def: &ModuleDecl, span: Span) -> Result<()> {
        for ann in &module_def.annotations {
            self.validate_annotation_target_usage(ann, AnnotationTargetKind::Module, span)?;
        }

        let module_path = self.current_module_path_for(&module_def.name);
        self.module_scope_stack.push(module_path.clone());

        let mut module_items = module_def.items.clone();
        if self.execute_module_comptime_handlers(module_def, &module_path, &mut module_items)? {
            self.module_scope_stack.pop();
            return Ok(());
        }
        if self.execute_module_inline_comptime_blocks(&module_path, &mut module_items)? {
            self.module_scope_stack.pop();
            return Ok(());
        }

        let mut qualified_items = Vec::with_capacity(module_items.len());
        for inner in &module_items {
            qualified_items.push(self.qualify_module_item(inner, &module_path)?);
        }

        for qualified in &qualified_items {
            self.register_missing_module_functions(qualified)?;
        }

        for qualified in &qualified_items {
            self.compile_item_with_context(qualified, false)?;
        }

        let exports = self.collect_module_runtime_exports(&module_items, &module_path);
        let entries: Vec<ObjectEntry> = exports
            .into_iter()
            .map(|(name, value_ident)| ObjectEntry::Field {
                key: name,
                value: Expr::Identifier(value_ident, span),
                type_annotation: None,
            })
            .collect();
        let module_object = Expr::Object(entries, span);
        self.compile_expr(&module_object)?;

        let binding_idx = self.get_or_create_module_binding(&module_path);
        self.emit(Instruction::new(
            OpCode::StoreModuleBinding,
            Some(Operand::ModuleBinding(binding_idx)),
        ));
        self.propagate_initializer_type_to_slot(binding_idx, false, false);

        if self.module_scope_stack.len() == 1 {
            self.module_namespace_bindings
                .insert(module_def.name.clone());
        }

        self.emit_annotation_lifecycle_calls_for_module(
            &module_path,
            &module_def.annotations,
            Some(binding_idx),
        )?;

        self.module_scope_stack.pop();
        Ok(())
    }

    /// Compile a query (Backtest, Alert, or With/CTE).
    ///
    /// For CTE (WITH) queries:
    /// 1. Compile each CTE subquery and store the result in a named module_binding variable.
    /// 2. Compile the main query (which can reference CTEs by name as variables).
    ///
    /// For Backtest and Alert queries, emit a stub for now.
    fn compile_query(&mut self, query: &Query) -> Result<()> {
        match query {
            Query::With(with_query) => {
                // Compile each CTE: evaluate its subquery and store as a named variable
                for cte in &with_query.ctes {
                    // Recursively compile the CTE's subquery
                    self.compile_query(&cte.query)?;

                    // Store the result in a module_binding variable with the CTE's name
                    let binding_idx = self.get_or_create_module_binding(&cte.name);
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }

                // Compile the main query
                self.compile_query(&with_query.query)?;
            }
            Query::Backtest(_backtest) => {
                // Backtest queries require runtime context to evaluate.
                // Push null as placeholder — the runtime executor handles backtest
                // execution when given a full ExecutionContext.
                self.emit(Instruction::simple(OpCode::PushNull));
            }
            Query::Alert(alert) => {
                // Compile alert condition
                self.compile_expr(&alert.condition)?;
                // Push null as placeholder (alert evaluation requires runtime context)
                self.emit(Instruction::simple(OpCode::Pop));
                self.emit(Instruction::simple(OpCode::PushNull));
            }
        }
        Ok(())
    }

    pub(super) fn propagate_initializer_type_to_slot(&mut self, slot: u16, is_local: bool, _is_mutable: bool) {
        self.propagate_assignment_type_to_slot(slot, is_local, true);
    }

    /// Compile a statement
    pub(super) fn compile_statement(&mut self, stmt: &Statement) -> Result<()> {
        match stmt {
            Statement::Return(expr_opt, _span) => {
                // Prevent returning references — refs are scoped borrows
                // that cannot escape the function (would create dangling refs).
                if let Some(expr) = expr_opt {
                    if let Expr::Reference { span: ref_span, .. } = expr {
                        return Err(ShapeError::SemanticError {
                            message: "cannot return a reference — references are scoped borrows that cannot escape the function. Return an owned value instead".to_string(),
                            location: Some(self.span_to_source_location(*ref_span)),
                        });
                    }
                    // Note: returning a ref_local identifier is allowed — compile_expr
                    // emits DerefLoad which returns the dereferenced *value*, not the
                    // reference itself. Only returning `&x` (Expr::Reference) is blocked.
                    self.compile_expr(expr)?;
                } else {
                    self.emit(Instruction::simple(OpCode::PushNull));
                }
                // Emit drops for all active drop scopes before returning
                let total_scopes = self.drop_locals.len();
                if total_scopes > 0 {
                    self.emit_drops_for_early_exit(total_scopes)?;
                }
                self.emit(Instruction::simple(OpCode::ReturnValue));
            }

            Statement::Break(_) => {
                let in_loop = !self.loop_stack.is_empty();
                if in_loop {
                    // Emit drops for drop scopes inside the loop before breaking
                    let scopes_to_exit = self
                        .loop_stack
                        .last()
                        .map(|ctx| self.drop_locals.len().saturating_sub(ctx.drop_scope_depth))
                        .unwrap_or(0);
                    if scopes_to_exit > 0 {
                        self.emit_drops_for_early_exit(scopes_to_exit)?;
                    }
                    let jump_idx = self.emit_jump(OpCode::Jump, 0);
                    if let Some(loop_ctx) = self.loop_stack.last_mut() {
                        loop_ctx.break_jumps.push(jump_idx);
                    }
                } else {
                    return Err(ShapeError::RuntimeError {
                        message: "break statement outside of loop".to_string(),
                        location: None,
                    });
                }
            }

            Statement::Continue(_) => {
                if let Some(loop_ctx) = self.loop_stack.last() {
                    // Copy values we need before mutable borrow
                    let scopes_to_exit = self
                        .drop_locals
                        .len()
                        .saturating_sub(loop_ctx.drop_scope_depth);
                    let continue_target = loop_ctx.continue_target;
                    // Emit drops for drop scopes inside the loop before continuing
                    if scopes_to_exit > 0 {
                        self.emit_drops_for_early_exit(scopes_to_exit)?;
                    }
                    let offset = continue_target as i32 - self.program.current_offset() as i32 - 1;
                    self.emit(Instruction::new(
                        OpCode::Jump,
                        Some(Operand::Offset(offset)),
                    ));
                } else {
                    return Err(ShapeError::RuntimeError {
                        message: "continue statement outside of loop".to_string(),
                        location: None,
                    });
                }
            }

            Statement::VariableDecl(var_decl, _) => {
                // Set pending variable name for hoisting integration.
                // compile_typed_object_literal uses self to include hoisted fields in the schema.
                self.pending_variable_name =
                    var_decl.pattern.as_identifier().map(|s| s.to_string());

                // Compile-time range check: if the type annotation is a width type
                // (i8, u8, i16, etc.) and the initializer is a constant expression,
                // verify the value fits in the declared width.
                if let (Some(type_ann), Some(init_expr)) =
                    (&var_decl.type_annotation, &var_decl.value)
                {
                    if let shape_ast::ast::TypeAnnotation::Basic(type_name) = type_ann {
                        if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
                            if let Some(const_val) =
                                crate::compiler::expressions::function_calls::eval_const_expr_to_nanboxed(init_expr)
                            {
                                let in_range = if let Some(i) = const_val.as_i64() {
                                    w.in_range_i64(i)
                                } else if let Some(f) = const_val.as_f64() {
                                    // Float → int truncation check
                                    let i = f as i64;
                                    (i as f64 == f) && w.in_range_i64(i)
                                } else {
                                    true // non-numeric, let runtime handle it
                                };
                                if !in_range {
                                    return Err(shape_ast::error::ShapeError::SemanticError {
                                        message: format!(
                                            "value does not fit in `{}` (range {}..={})",
                                            type_name,
                                            w.min_value(),
                                            w.max_value()
                                        ),
                                        location: Some(self.span_to_source_location(shape_ast::ast::Spanned::span(init_expr))),
                                    });
                                }
                            }
                        }
                    }
                }

                // Compile initializer — register the variable even if the initializer fails,
                // to prevent cascading "Undefined variable" errors on later references.
                let init_err = if let Some(init_expr) = &var_decl.value {
                    // Special handling: Table row literal syntax
                    // `let t: Table<T> = [a, b], [c, d]` → compile as table construction
                    if let Expr::TableRows(rows, tr_span) = init_expr {
                        match self.compile_table_rows(rows, &var_decl.type_annotation, *tr_span) {
                            Ok(()) => None,
                            Err(e) => {
                                self.emit(Instruction::simple(OpCode::PushNull));
                                Some(e)
                            }
                        }
                    } else {
                        match self.compile_expr(init_expr) {
                            Ok(()) => None,
                            Err(e) => {
                                self.emit(Instruction::simple(OpCode::PushNull));
                                Some(e)
                            }
                        }
                    }
                } else {
                    self.emit(Instruction::simple(OpCode::PushNull));
                    None
                };

                // Clear pending variable name after init expression is compiled
                self.pending_variable_name = None;

                // Emit BindSchema for Table<T> annotations (runtime safety net)
                if let Some(ref type_ann) = var_decl.type_annotation {
                    if let Some(schema_id) = self.get_table_schema_id(type_ann) {
                        self.emit(Instruction::new(
                            OpCode::BindSchema,
                            Some(Operand::Count(schema_id)),
                        ));
                    }
                }

                // At top-level (no current function), create module_bindings; otherwise create locals
                if self.current_function.is_none() {
                    // Top-level: create module_binding variable
                    if let Some(name) = var_decl.pattern.as_identifier() {
                        let binding_idx = self.get_or_create_module_binding(name);

                        // Emit StoreModuleBindingTyped for width-typed bindings,
                        // otherwise emit regular StoreModuleBinding.
                        let used_typed_store = if let Some(TypeAnnotation::Basic(type_name)) =
                            var_decl.type_annotation.as_ref()
                        {
                            if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
                                self.emit(Instruction::new(
                                    OpCode::StoreModuleBindingTyped,
                                    Some(Operand::TypedModuleBinding(
                                        binding_idx,
                                        crate::bytecode::NumericWidth::from_int_width(w),
                                    )),
                                ));
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if !used_typed_store {
                            self.emit(Instruction::new(
                                OpCode::StoreModuleBinding,
                                Some(Operand::ModuleBinding(binding_idx)),
                            ));
                        }

                        // Track const module bindings for reassignment checks
                        if var_decl.kind == VarKind::Const {
                            self.const_module_bindings.insert(binding_idx);
                        }

                        // Track immutable `let` bindings at module level
                        if var_decl.kind == VarKind::Let && !var_decl.is_mut {
                            self.immutable_module_bindings.insert(binding_idx);
                        }

                        // Track type annotation if present (for type checker)
                        if let Some(ref type_ann) = var_decl.type_annotation {
                            if let Some(type_name) =
                                Self::tracked_type_name_from_annotation(type_ann)
                            {
                                self.set_module_binding_type_info(binding_idx, &type_name);
                            }
                            // Handle Table<T> generic annotation
                            self.try_track_datatable_type(type_ann, binding_idx, false)?;
                        } else {
                            let is_mutable = var_decl.kind == shape_ast::ast::VarKind::Var;
                            self.propagate_initializer_type_to_slot(binding_idx, false, is_mutable);
                        }

                        // Track for auto-drop at program exit
                        let binding_type_name = self
                            .type_tracker
                            .get_binding_type(binding_idx)
                            .and_then(|info| info.type_name.clone());
                        let drop_kind = binding_type_name
                            .as_ref()
                            .and_then(|tn| self.drop_type_info.get(tn).copied())
                            .or_else(|| {
                                var_decl
                                    .type_annotation
                                    .as_ref()
                                    .and_then(|ann| self.annotation_drop_kind(ann))
                            });
                        if drop_kind.is_some() {
                            let is_async = match drop_kind {
                                Some(DropKind::AsyncOnly) => true,
                                Some(DropKind::Both) => false,
                                Some(DropKind::SyncOnly) | None => false,
                            };
                            self.track_drop_module_binding(binding_idx, is_async);
                        }
                        if let Some(value) = &var_decl.value {
                            self.update_reference_binding_from_expr(binding_idx, false, value);
                        } else {
                            self.clear_reference_binding(binding_idx, false);
                        }
                    } else {
                        self.compile_destructure_pattern_global(&var_decl.pattern)?;
                    }
                } else {
                    // Inside function: create local variable
                    self.compile_destructure_pattern(&var_decl.pattern)?;

                    // Patch StoreLocal → StoreLocalTyped for width-typed simple bindings.
                    // compile_destructure_pattern emits StoreLocal(idx) for Identifier patterns;
                    // we upgrade it here when the type annotation is a width type.
                    if let (Some(name), Some(TypeAnnotation::Basic(type_name))) = (
                        var_decl.pattern.as_identifier(),
                        var_decl.type_annotation.as_ref(),
                    ) {
                        if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
                            if let Some(local_idx) = self.resolve_local(name) {
                                if let Some(last) = self.program.instructions.last_mut() {
                                    if last.opcode == OpCode::StoreLocal {
                                        last.opcode = OpCode::StoreLocalTyped;
                                        last.operand = Some(Operand::TypedLocal(
                                            local_idx,
                                            crate::bytecode::NumericWidth::from_int_width(w),
                                        ));
                                    }
                                }
                            }
                        }
                    }

                    // Track const locals for reassignment checks
                    if var_decl.kind == VarKind::Const {
                        if let Some(name) = var_decl.pattern.as_identifier() {
                            if let Some(local_idx) = self.resolve_local(name) {
                                self.const_locals.insert(local_idx);
                            }
                        }
                    }

                    // Track immutable `let` bindings (not `let mut` and not `var`)
                    // `let` without `mut` is immutable by default.
                    // `var` is always mutable (inferred from usage).
                    // `let mut` is explicitly mutable.
                    if var_decl.kind == VarKind::Let && !var_decl.is_mut {
                        if let Some(name) = var_decl.pattern.as_identifier() {
                            if let Some(local_idx) = self.resolve_local(name) {
                                self.immutable_locals.insert(local_idx);
                            }
                        }
                    }

                    // Track type annotation first (so drop tracking can resolve the type)
                    if let Some(name) = var_decl.pattern.as_identifier() {
                        if let Some(ref type_ann) = var_decl.type_annotation {
                            if let Some(type_name) =
                                Self::tracked_type_name_from_annotation(type_ann)
                            {
                                // Get the local index for self variable
                                if let Some(local_idx) = self.resolve_local(name) {
                                    self.set_local_type_info(local_idx, &type_name);
                                }
                            }
                            // Handle Table<T> generic annotation
                            if let Some(local_idx) = self.resolve_local(name) {
                                self.try_track_datatable_type(type_ann, local_idx, true)?;
                            }
                        } else if let Some(local_idx) = self.resolve_local(name) {
                            let is_mutable = var_decl.kind == shape_ast::ast::VarKind::Var;
                            self.propagate_initializer_type_to_slot(local_idx, true, is_mutable);
                        }
                    }

                    // Track for auto-drop at scope exit (DropCall silently skips non-Drop types).
                    // Select sync vs async opcode based on the type's DropKind.
                    if let Some(name) = var_decl.pattern.as_identifier() {
                        if let Some(local_idx) = self.resolve_local(name) {
                            let drop_kind = self.local_drop_kind(local_idx).or_else(|| {
                                var_decl
                                    .type_annotation
                                    .as_ref()
                                    .and_then(|ann| self.annotation_drop_kind(ann))
                            });

                            let is_async = match drop_kind {
                                Some(DropKind::AsyncOnly) => {
                                    if !self.current_function_is_async {
                                        let tn = self
                                            .type_tracker
                                            .get_local_type(local_idx)
                                            .and_then(|info| info.type_name.clone())
                                            .unwrap_or_else(|| name.to_string());
                                        return Err(ShapeError::SemanticError {
                                            message: format!(
                                                "type '{}' has only an async drop() and cannot be used in a sync context; \
                                                 add a sync method drop(self) or use it inside an async function",
                                                tn
                                            ),
                                            location: None,
                                        });
                                    }
                                    true
                                }
                                Some(DropKind::Both) => self.current_function_is_async,
                                Some(DropKind::SyncOnly) | None => false,
                            };
                            self.track_drop_local(local_idx, is_async);
                            if let Some(value) = &var_decl.value {
                                self.update_reference_binding_from_expr(local_idx, true, value);
                            } else {
                                self.clear_reference_binding(local_idx, true);
                            }
                        }
                    }
                }

                if let Some(e) = init_err {
                    return Err(e);
                }
            }

            Statement::Assignment(assign, _) => 'assign: {
                // Check for const reassignment
                if let Some(name) = assign.pattern.as_identifier() {
                    if let Some(local_idx) = self.resolve_local(name) {
                        if self.const_locals.contains(&local_idx) {
                            return Err(ShapeError::SemanticError {
                                message: format!("Cannot reassign const variable '{}'", name),
                                location: None,
                            });
                        }
                        // Check for immutable `let` reassignment
                        if self.immutable_locals.contains(&local_idx) {
                            return Err(ShapeError::SemanticError {
                                message: format!(
                                    "Cannot reassign immutable variable '{}'. Use `let mut` or `var` for mutable bindings",
                                    name
                                ),
                                location: None,
                            });
                        }
                        self.borrow_checker
                            .check_write_allowed(
                                Self::borrow_key_for_local(local_idx),
                                None,
                            )
                            .map_err(|e| match e {
                                ShapeError::SemanticError { message, location } => {
                                    let user_msg = message.replace(
                                        &format!("(slot {})", local_idx),
                                        &format!("'{}'", name),
                                    );
                                    ShapeError::SemanticError {
                                        message: user_msg,
                                        location,
                                    }
                                }
                                other => other,
                            })?;
                    } else {
                        let scoped_name = self
                            .resolve_scoped_module_binding_name(name)
                            .unwrap_or_else(|| name.to_string());
                        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                            if self.const_module_bindings.contains(&binding_idx) {
                                return Err(ShapeError::SemanticError {
                                    message: format!("Cannot reassign const variable '{}'", name),
                                    location: None,
                                });
                            }
                            // Check for immutable `let` reassignment at module level
                            if self.immutable_module_bindings.contains(&binding_idx) {
                                return Err(ShapeError::SemanticError {
                                    message: format!(
                                        "Cannot reassign immutable variable '{}'. Use `let mut` or `var` for mutable bindings",
                                        name
                                    ),
                                    location: None,
                                });
                            }
                            self.borrow_checker
                                .check_write_allowed(
                                    Self::borrow_key_for_module_binding(binding_idx),
                                    None,
                                )
                                .map_err(|e| match e {
                                    ShapeError::SemanticError { message, location } => {
                                        let user_msg = message.replace(
                                            &format!(
                                                "(slot {})",
                                                Self::borrow_key_for_module_binding(binding_idx)
                                            ),
                                            &format!("'{}'", name),
                                        );
                                        ShapeError::SemanticError {
                                            message: user_msg,
                                            location,
                                        }
                                    }
                                    other => other,
                                })?;
                        }
                    }
                }

                // Optimization: x = x.push(val) → ArrayPushLocal (O(1) in-place mutation)
                if let Some(name) = assign.pattern.as_identifier() {
                    if let Expr::MethodCall {
                        receiver,
                        method,
                        args,
                        ..
                    } = &assign.value
                    {
                        if method == "push" && args.len() == 1 {
                            if let Expr::Identifier(recv_name, _) = receiver.as_ref() {
                                if recv_name == name {
                                    if let Some(local_idx) = self.resolve_local(name) {
                                        self.compile_expr(&args[0])?;
                                        let pushed_numeric = self.last_expr_numeric_type;
                                        self.emit(Instruction::new(
                                            OpCode::ArrayPushLocal,
                                            Some(Operand::Local(local_idx)),
                                        ));
                                        if let Some(numeric_type) = pushed_numeric {
                                            self.mark_slot_as_numeric_array(
                                                local_idx,
                                                true,
                                                numeric_type,
                                            );
                                        }
                                        break 'assign;
                                    } else {
                                        let binding_idx = self.get_or_create_module_binding(name);
                                        self.compile_expr(&args[0])?;
                                        let pushed_numeric = self.last_expr_numeric_type;
                                        self.emit(Instruction::new(
                                            OpCode::ArrayPushLocal,
                                            Some(Operand::ModuleBinding(binding_idx)),
                                        ));
                                        if let Some(numeric_type) = pushed_numeric {
                                            self.mark_slot_as_numeric_array(
                                                binding_idx,
                                                false,
                                                numeric_type,
                                            );
                                        }
                                        break 'assign;
                                    }
                                }
                            }
                        }
                    }
                }

                // Compile value
                self.compile_expr(&assign.value)?;
                let assigned_ident = assign.pattern.as_identifier().map(str::to_string);

                // Store in variable
                self.compile_destructure_assignment(&assign.pattern)?;
                if let Some(name) = assigned_ident.as_deref() {
                    if let Some(local_idx) = self.resolve_local(name) {
                        if !self.ref_locals.contains(&local_idx) {
                            self.update_reference_binding_from_expr(local_idx, true, &assign.value);
                        }
                    } else if let Some(scoped_name) =
                        self.resolve_scoped_module_binding_name(name)
                    {
                        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                            self.update_reference_binding_from_expr(
                                binding_idx,
                                false,
                                &assign.value,
                            );
                        }
                    }
                    self.propagate_assignment_type_to_identifier(name);
                }
            }

            Statement::Expression(expr, _) => {
                // Fast path: arr.push(val) as standalone statement → in-place mutation
                // (avoids the LoadLocal+Pop overhead from the expression-level optimization)
                if let Expr::MethodCall {
                    receiver,
                    method,
                    args,
                    ..
                } = expr
                {
                    if method == "push" && args.len() == 1 {
                        if let Expr::Identifier(recv_name, _) = receiver.as_ref() {
                            if let Some(local_idx) = self.resolve_local(recv_name) {
                                self.compile_expr(&args[0])?;
                                let pushed_numeric = self.last_expr_numeric_type;
                                self.emit(Instruction::new(
                                    OpCode::ArrayPushLocal,
                                    Some(Operand::Local(local_idx)),
                                ));
                                if let Some(numeric_type) = pushed_numeric {
                                    self.mark_slot_as_numeric_array(
                                        local_idx,
                                        true,
                                        numeric_type,
                                    );
                                }
                                return Ok(());
                            } else if !self
                                .mutable_closure_captures
                                .contains_key(recv_name.as_str())
                            {
                                let binding_idx = self.get_or_create_module_binding(recv_name);
                                self.compile_expr(&args[0])?;
                                self.emit(Instruction::new(
                                    OpCode::ArrayPushLocal,
                                    Some(Operand::ModuleBinding(binding_idx)),
                                ));
                                return Ok(());
                            }
                        }
                    }
                }
                self.compile_expr(expr)?;
                self.emit(Instruction::simple(OpCode::Pop));
            }

            Statement::For(for_loop, _) => {
                self.compile_for_loop(for_loop)?;
            }

            Statement::While(while_loop, _) => {
                self.compile_while_loop(while_loop)?;
            }

            Statement::If(if_stmt, _) => {
                self.compile_if_statement(if_stmt)?;
            }
            Statement::Extend(extend, span) => {
                if !self.comptime_mode {
                    return Err(ShapeError::SemanticError {
                        message:
                            "`extend` as a statement is only valid inside `comptime { }` context"
                                .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                self.emit_comptime_extend_directive(extend, *span)?;
            }
            Statement::RemoveTarget(span) => {
                if !self.comptime_mode {
                    return Err(ShapeError::SemanticError {
                        message: "`remove target` is only valid inside `comptime { }` context"
                            .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                self.emit_comptime_remove_directive(*span)?;
            }
            Statement::SetParamType {
                param_name,
                type_annotation,
                span,
            } => {
                if !self.comptime_mode {
                    return Err(ShapeError::SemanticError {
                        message: "`set param` is only valid inside `comptime { }` context"
                            .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                self.emit_comptime_set_param_type_directive(param_name, type_annotation, *span)?;
            }
            Statement::SetParamValue {
                param_name,
                expression,
                span,
            } => {
                if !self.comptime_mode {
                    return Err(ShapeError::SemanticError {
                        message: "`set param` is only valid inside `comptime { }` context"
                            .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                self.emit_comptime_set_param_value_directive(param_name, expression, *span)?;
            }
            Statement::SetReturnType {
                type_annotation,
                span,
            } => {
                if !self.comptime_mode {
                    return Err(ShapeError::SemanticError {
                        message: "`set return` is only valid inside `comptime { }` context"
                            .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                self.emit_comptime_set_return_type_directive(type_annotation, *span)?;
            }
            Statement::SetReturnExpr { expression, span } => {
                if !self.comptime_mode {
                    return Err(ShapeError::SemanticError {
                        message: "`set return` is only valid inside `comptime { }` context"
                            .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                self.emit_comptime_set_return_expr_directive(expression, *span)?;
            }
            Statement::ReplaceBody { body, span } => {
                if !self.comptime_mode {
                    return Err(ShapeError::SemanticError {
                        message: "`replace body` is only valid inside `comptime { }` context"
                            .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                self.emit_comptime_replace_body_directive(body, *span)?;
            }
            Statement::ReplaceBodyExpr { expression, span } => {
                if !self.comptime_mode {
                    return Err(ShapeError::SemanticError {
                        message: "`replace body` is only valid inside `comptime { }` context"
                            .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                self.emit_comptime_replace_body_expr_directive(expression, *span)?;
            }
            Statement::ReplaceModuleExpr { expression, span } => {
                if !self.comptime_mode {
                    return Err(ShapeError::SemanticError {
                        message: "`replace module` is only valid inside `comptime { }` context"
                            .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                self.emit_comptime_replace_module_expr_directive(expression, *span)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler::BytecodeCompiler;
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_ast::parser::parse_program;

    #[test]
    fn test_module_decl_function_resolves_module_const() {
        let code = r#"
            mod math {
                const BASE = 21
                fn twice() {
                    BASE * 2
                }
            }
            math.twice()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.populate_module_objects();
        let result = vm.execute(None).expect("Failed to execute");
        assert_eq!(
            result
                .as_number_coerce()
                .expect("module call should return number"),
            42.0
        );
    }

    #[test]
    fn test_module_annotation_can_replace_module_items() {
        let code = r#"
            annotation synth_module() {
                targets: [module]
                comptime post(target, ctx) {
                    replace module ("const ANSWER = 40; fn plus_two() { ANSWER + 2 }")
                }
            }

            @synth_module()
            mod demo {}

            demo.plus_two()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.populate_module_objects();
        let result = vm.execute(None).expect("Failed to execute");
        assert_eq!(
            result
                .as_number_coerce()
                .expect("module call should return number"),
            42.0
        );
    }

    #[test]
    fn test_module_inline_comptime_can_replace_module_items() {
        let code = r#"
            mod demo {
                comptime {
                    replace module ("const ANSWER = 40; fn plus_two() { ANSWER + 2 }")
                }
            }

            demo.plus_two()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.populate_module_objects();
        let result = vm.execute(None).expect("Failed to execute");
        assert_eq!(
            result
                .as_number_coerce()
                .expect("module call should return number"),
            42.0
        );
    }

    #[test]
    fn test_module_inline_comptime_can_use_module_local_comptime_helper() {
        let code = r#"
            mod demo {
                comptime fn synth() {
                    "const ANSWER = 40; fn plus_two() { ANSWER + 2 }"
                }

                comptime {
                    replace module (synth())
                }
            }

            demo.plus_two()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.populate_module_objects();
        let result = vm.execute(None).expect("Failed to execute");
        assert_eq!(
            result
                .as_number_coerce()
                .expect("module call should return number"),
            42.0
        );
    }

    #[test]
    fn test_type_annotated_variable_no_wrapping() {
        // BUG-1/BUG-2 fix: variable declarations must NOT emit WrapTypeAnnotation
        // (the wrapper broke arithmetic and comparisons)
        let code = r#"
            type Currency = Number
            let x: Currency = 123
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        // WrapTypeAnnotation should NOT be emitted for variable declarations
        let has_wrap_instruction = bytecode
            .instructions
            .iter()
            .any(|instr| instr.opcode == crate::bytecode::OpCode::WrapTypeAnnotation);
        assert!(
            !has_wrap_instruction,
            "Should NOT emit WrapTypeAnnotation for type-annotated variable"
        );
    }

    #[test]
    fn test_untyped_variable_no_wrapping() {
        // Variables without type annotations should NOT emit WrapTypeAnnotation
        let code = r#"
            let x = 123
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        // Check that WrapTypeAnnotation instruction was NOT emitted
        let has_wrap_instruction = bytecode
            .instructions
            .iter()
            .any(|instr| instr.opcode == crate::bytecode::OpCode::WrapTypeAnnotation);
        assert!(
            !has_wrap_instruction,
            "Should NOT emit WrapTypeAnnotation for untyped variable"
        );
    }

    // ===== Phase 2: Extend Block Compilation Tests =====

    #[test]
    fn test_extend_block_compiles() {
        let code = r#"
            extend Number {
                method double() {
                    return self * 2
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse extend block");
        let bytecode = BytecodeCompiler::new().compile(&program);
        assert!(
            bytecode.is_ok(),
            "Extend block should compile: {:?}",
            bytecode.err()
        );

        // Verify a function named "Number.double" was generated (qualified extend name).
        let bytecode = bytecode.unwrap();
        let has_double = bytecode.functions.iter().any(|f| f.name == "Number.double");
        assert!(
            has_double,
            "Should generate 'Number.double' function from extend block"
        );
    }

    #[test]
    fn test_extend_method_has_self_param() {
        let code = r#"
            extend Number {
                method add(n) {
                    return self + n
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let func = bytecode.functions.iter().find(|f| f.name == "Number.add");
        assert!(func.is_some(), "Should have 'Number.add' function");
        // The function should have 2 params: self + n
        assert_eq!(
            func.unwrap().arity,
            2,
            "add() should have arity 2 (self + n)"
        );
    }

    #[test]
    fn test_extend_method_rejects_explicit_self_param() {
        let code = r#"
            extend Number {
                method add(self, n) {
                    return self + n
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("Compiler should reject explicit self receiver param in methods");
        let msg = format!("{err}");
        assert!(
            msg.contains("explicit `self` parameter"),
            "Expected explicit self error, got: {msg}"
        );
    }

    // ===== Phase 3: Annotation Handler Compilation Tests =====

    #[test]
    fn test_annotation_def_compiles_handlers() {
        let code = r#"
            annotation warmup(period) {
                before(args, ctx) {
                    args
                }
                after(args, result, ctx) {
                    result
                }
            }
            function test() { return 42; }
        "#;
        let program = parse_program(code).expect("Failed to parse annotation def");
        let bytecode = BytecodeCompiler::new().compile(&program);
        assert!(
            bytecode.is_ok(),
            "Annotation def should compile: {:?}",
            bytecode.err()
        );

        let bytecode = bytecode.unwrap();
        // Verify CompiledAnnotation was registered
        assert!(
            bytecode.compiled_annotations.contains_key("warmup"),
            "Should have compiled 'warmup' annotation"
        );

        let compiled = bytecode.compiled_annotations.get("warmup").unwrap();
        assert!(
            compiled.before_handler.is_some(),
            "Should have before handler"
        );
        assert!(
            compiled.after_handler.is_some(),
            "Should have after handler"
        );
    }

    #[test]
    fn test_annotation_handler_function_names() {
        let code = r#"
            annotation my_ann(x) {
                before(args, ctx) {
                    args
                }
            }
            function test() { return 1; }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        // Handler should be compiled as an internal function
        let compiled = bytecode.compiled_annotations.get("my_ann").unwrap();
        let handler_id = compiled.before_handler.unwrap() as usize;
        assert!(
            handler_id < bytecode.functions.len(),
            "Handler function ID should be valid"
        );

        let handler_fn = &bytecode.functions[handler_id];
        assert_eq!(
            handler_fn.name, "my_ann___before",
            "Handler function should be named my_ann___before"
        );
    }

    // ===== Phase 4: Compile-Time Function Wrapping Tests =====

    #[test]
    fn test_annotated_function_generates_wrapper() {
        let code = r#"
            annotation tracked(label) {
                before(args, ctx) {
                    args
                }
            }
            @tracked("my_func")
            function compute(x) {
                return x * 2
            }
            function test() { return 1; }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new().compile(&program);
        assert!(
            bytecode.is_ok(),
            "Annotated function should compile: {:?}",
            bytecode.err()
        );

        let bytecode = bytecode.unwrap();
        // Should have the original function (wrapper) and the impl
        let has_impl = bytecode
            .functions
            .iter()
            .any(|f| f.name == "compute___impl");
        assert!(has_impl, "Should generate compute___impl function");

        let has_wrapper = bytecode.functions.iter().any(|f| f.name == "compute");
        assert!(has_wrapper, "Should keep compute as wrapper");
    }

    #[test]
    fn test_unannotated_function_no_wrapper() {
        let code = r#"
            function plain(x) {
                return x + 1
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        // Should NOT have an ___impl function
        let has_impl = bytecode
            .functions
            .iter()
            .any(|f| f.name.ends_with("___impl"));
        assert!(
            !has_impl,
            "Non-annotated function should not generate ___impl"
        );
    }

    // ===== Sprint 10: Annotation chaining and target validation =====

    #[test]
    fn test_annotation_chaining_generates_chain() {
        // Two annotations on the same function should generate chained wrappers
        let code = r#"
            annotation first() {
                before(args, ctx) {
                    return args
                }
            }

            annotation second() {
                before(args, ctx) {
                    return args
                }
            }

            @first
            @second
            function compute(x) {
                return x * 2
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new().compile(&program);
        assert!(
            bytecode.is_ok(),
            "Chained annotations should compile: {:?}",
            bytecode.err()
        );
        let bytecode = bytecode.unwrap();

        // Should have: compute (outermost wrapper), compute___impl (body), compute___second (intermediate)
        let has_impl = bytecode
            .functions
            .iter()
            .any(|f| f.name == "compute___impl");
        assert!(has_impl, "Should generate compute___impl function");
        let has_wrapper = bytecode.functions.iter().any(|f| f.name == "compute");
        assert!(has_wrapper, "Should keep compute as outermost wrapper");
        let has_intermediate = bytecode
            .functions
            .iter()
            .any(|f| f.name == "compute___second");
        assert!(
            has_intermediate,
            "Should generate compute___second intermediate wrapper"
        );
    }

    #[test]
    fn test_annotation_allowed_targets_inferred() {
        // An annotation with before/after should have allowed_targets = [Function]
        let code = r#"
            annotation traced() {
                before(args, ctx) {
                    return args
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new().compile(&program).expect("compile");
        let ann = bytecode
            .compiled_annotations
            .get("traced")
            .expect("traced annotation");
        assert!(
            !ann.allowed_targets.is_empty(),
            "before handler should restrict targets"
        );
        assert!(
            ann.allowed_targets
                .contains(&shape_ast::ast::functions::AnnotationTargetKind::Function),
            "before handler should allow Function target"
        );
    }

    #[test]
    fn test_annotation_allowed_targets_explicit_override() {
        // Explicit `targets: [...]` should override inferred defaults.
        let code = r#"
            annotation traced() {
                targets: [type]
                before(args, ctx) {
                    return args
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new().compile(&program).expect("compile");
        let ann = bytecode
            .compiled_annotations
            .get("traced")
            .expect("traced annotation");
        assert_eq!(
            ann.allowed_targets,
            vec![shape_ast::ast::functions::AnnotationTargetKind::Type]
        );
    }

    #[test]
    fn test_metadata_only_annotation_defaults_to_definition_targets() {
        // An annotation with only metadata handler should default to definition targets.
        let code = r#"
            annotation info() {
                metadata() {
                    return { version: 1 }
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new().compile(&program).expect("compile");
        let ann = bytecode
            .compiled_annotations
            .get("info")
            .expect("info annotation");
        assert_eq!(
            ann.allowed_targets,
            vec![
                shape_ast::ast::functions::AnnotationTargetKind::Function,
                shape_ast::ast::functions::AnnotationTargetKind::Type,
                shape_ast::ast::functions::AnnotationTargetKind::Module
            ],
            "metadata-only annotation should default to definition targets"
        );
    }

    #[test]
    fn test_definition_lifecycle_targets_reject_expression_target() {
        let code = r#"
            annotation info() {
                targets: [expression]
                metadata(target, ctx) {
                    target.name
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("metadata hooks on expression targets should fail");
        let msg = format!("{}", err);
        assert!(
            msg.contains("not a definition target"),
            "expected definition-target restriction error, got: {}",
            msg
        );
    }

    #[test]
    fn test_annotation_target_validation_on_struct_type() {
        // Function-only annotation applied to a type should fail.
        let code = r#"
            annotation traced() {
                before(args, ctx) { return args }
            }

            @traced()
            type Point { x: int }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("function-only annotation on type should fail");
        let msg = format!("{}", err);
        assert!(
            msg.contains("cannot be applied to a type"),
            "expected type target validation error, got: {}",
            msg
        );
    }

    #[test]
    fn test_type_c_emits_native_layout_metadata() {
        let bytecode = compiles_to(
            r#"
            type C Pair32 {
                left: i32,
                right: i32,
            }
            "#,
        );

        assert_eq!(bytecode.native_struct_layouts.len(), 1);
        let layout = &bytecode.native_struct_layouts[0];
        assert_eq!(layout.name, "Pair32");
        assert_eq!(layout.abi, "C");
        assert_eq!(layout.size, 8);
        assert_eq!(layout.align, 4);
        assert_eq!(layout.fields.len(), 2);
        assert_eq!(layout.fields[0].name, "left");
        assert_eq!(layout.fields[0].offset, 0);
        assert_eq!(layout.fields[0].size, 4);
        assert_eq!(layout.fields[1].name, "right");
        assert_eq!(layout.fields[1].offset, 4);
        assert_eq!(layout.fields[1].size, 4);
    }

    #[test]
    fn test_type_c_auto_generates_into_from_traits() {
        let bytecode = compiles_to(
            r#"
            type C QuoteC {
                bid: i64,
                ask: i64,
            }

            type Quote {
                bid: i64,
                ask: i64,
            }
            "#,
        );

        let c_to_shape =
            bytecode.lookup_trait_method_symbol("Into", "QuoteC", Some("Quote"), "into");
        let shape_to_c =
            bytecode.lookup_trait_method_symbol("Into", "Quote", Some("QuoteC"), "into");
        let from_c = bytecode.lookup_trait_method_symbol("From", "Quote", Some("QuoteC"), "from");
        let from_shape =
            bytecode.lookup_trait_method_symbol("From", "QuoteC", Some("Quote"), "from");

        assert!(c_to_shape.is_some(), "expected Into<Quote> for QuoteC");
        assert!(shape_to_c.is_some(), "expected Into<QuoteC> for Quote");
        assert!(from_c.is_some(), "expected From<QuoteC> for Quote");
        assert!(from_shape.is_some(), "expected From<Quote> for QuoteC");
    }

    #[test]
    fn test_type_c_auto_conversion_function_compiles() {
        let _ = compiles_to(
            r#"
            type Quote {
                bid: i64,
                ask: i64,
            }

            type C QuoteC {
                bid: i64,
                ask: i64,
            }

            fn spread(q: QuoteC) -> i64 {
                let q_shape = __auto_native_from_QuoteC_to_Quote(q);
                q_shape.ask - q_shape.bid
            }

            spread(QuoteC { bid: 10, ask: 13 })
            "#,
        );
    }

    #[test]
    fn test_type_c_auto_conversion_rejects_incompatible_fields() {
        let program = parse_program(
            r#"
            type Price {
                value: i64,
            }

            type C PriceC {
                value: u64,
            }
            "#,
        )
        .expect("parse failed");
        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("incompatible type C conversion pair should fail");
        let msg = format!("{}", err);
        assert!(
            msg.contains("field type mismatch for auto conversion"),
            "expected type mismatch error, got: {}",
            msg
        );
    }

    // ===== Task 1: Meta on traits =====

    // ===== Drop Track: Sprint 2 Tests =====

    fn compiles_to(code: &str) -> crate::bytecode::BytecodeProgram {
        let program = parse_program(code).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        compiler.compile(&program).expect("compile failed")
    }

    // --- Permission checking tests ---

    #[test]
    fn test_extract_module_name() {
        assert_eq!(BytecodeCompiler::extract_module_name("file"), "file");
        assert_eq!(BytecodeCompiler::extract_module_name("std::file"), "file");
        assert_eq!(BytecodeCompiler::extract_module_name("std/io"), "io");
        assert_eq!(BytecodeCompiler::extract_module_name("a::b::c"), "c");
        assert_eq!(BytecodeCompiler::extract_module_name(""), "");
    }

    #[test]
    fn test_permission_check_allows_pure_module_imports() {
        // json is a pure module — should compile even with empty permissions
        let code = "from json use { parse }";
        let program = parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_permission_set(Some(shape_abi_v1::PermissionSet::pure()));
        // Should not fail — json requires no permissions
        let _result = compiler.compile(&program);
    }

    #[test]
    fn test_permission_check_blocks_file_import_under_pure() {
        let code = "from file use { read_text }";
        let program = parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_permission_set(Some(shape_abi_v1::PermissionSet::pure()));
        let result = compiler.compile(&program);
        assert!(
            result.is_err(),
            "Expected permission error for file::read_text under pure"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("Permission denied"),
            "Error should mention permission denied: {err_msg}"
        );
        assert!(
            err_msg.contains("fs.read"),
            "Error should mention fs.read: {err_msg}"
        );
    }

    #[test]
    fn test_permission_check_allows_file_import_with_fs_read() {
        let code = "from file use { read_text }";
        let program = parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        let pset = shape_abi_v1::PermissionSet::from_iter([shape_abi_v1::Permission::FsRead]);
        compiler.set_permission_set(Some(pset));
        // Should not fail
        let _result = compiler.compile(&program);
    }

    #[test]
    fn test_permission_check_no_permission_set_allows_everything() {
        // When permission_set is None (default), no checking is done
        let code = "from file use { read_text }";
        let program = parse_program(code).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        // permission_set is None by default — should compile fine
        let _result = compiler.compile(&program);
    }

    #[test]
    fn test_permission_check_namespace_import_blocked() {
        let code = "use http";
        let program = parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_permission_set(Some(shape_abi_v1::PermissionSet::pure()));
        let result = compiler.compile(&program);
        assert!(
            result.is_err(),
            "Expected permission error for `use http` under pure"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("Permission denied"),
            "Error should mention permission denied: {err_msg}"
        );
    }

    #[test]
    fn test_permission_check_namespace_import_allowed() {
        let code = "use http";
        let program = parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_permission_set(Some(shape_abi_v1::PermissionSet::full()));
        // Should not fail
        let _result = compiler.compile(&program);
    }
}
