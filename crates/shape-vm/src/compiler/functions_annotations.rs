//! Annotation lifecycle and comptime handler compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use shape_ast::ast::{
    DestructurePattern, Expr, FunctionDef, Literal, ObjectEntry, Span, Statement, VarKind,
    VariableDecl,
};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_schema::FieldType;
use shape_value::ValueWord;
use std::collections::{HashMap, HashSet};

use super::BytecodeCompiler;

impl BytecodeCompiler {
    pub(super) fn emit_annotation_lifecycle_calls(&mut self, func_def: &FunctionDef) -> Result<()> {
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
            let Some((_, compiled)) = self.lookup_compiled_annotation(ann) else {
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
    pub(super) fn execute_comptime_handlers(&mut self, func_def: &mut FunctionDef) -> Result<bool> {
        let mut removed = false;
        let annotations = func_def.annotations.clone();

        // Phase 1: comptime pre
        for ann in &annotations {
            if let Some((_, compiled)) = self.lookup_compiled_annotation(ann) {
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
                if let Some((_, compiled)) = self.lookup_compiled_annotation(ann) {
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
            Expr::QualifiedFunctionCall {
                namespace,
                function,
                args,
                named_args,
                ..
            } => {
                names.insert(format!("{}::{}", namespace, function));
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

    pub(super) fn process_comptime_directives_for_function(
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
    pub(super) fn validate_annotation_targets(&self, func_def: &FunctionDef) -> Result<()> {
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
    pub(super) fn find_compiled_annotations(
        &self,
        func_def: &FunctionDef,
    ) -> Vec<crate::bytecode::CompiledAnnotation> {
        let mut result = Vec::new();
        for ann in &func_def.annotations {
            if let Some((_, compiled)) = self.lookup_compiled_annotation(ann) {
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
    pub(super) fn compile_chained_annotations(
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
            let ann_arg_exprs =
                self.annotation_args_for_compiled_name(&func_def.annotations, &ann.name);

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
    pub(super) fn compile_wrapped_function(
        &mut self,
        func_def: &FunctionDef,
        compiled_ann: crate::bytecode::CompiledAnnotation,
    ) -> Result<()> {
        // Find the annotation on the function to get the arg expressions
        let ann = func_def
            .annotations
            .iter()
            .find(|a| self.annotation_matches_compiled_name(a, &compiled_ann.name))
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
    pub(super) fn compile_annotation_wrapper(
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
}
