//! Expression compilation
//!
//! This module contains the main expression compilation logic, organized by expression type.

use shape_ast::ast::{Expr, Span};
use shape_ast::error::{Result, ShapeError};

use super::BytecodeCompiler;
use crate::borrow_checker::BorrowMode;
use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use shape_runtime::type_schema::FieldType;

/// Extract the span from an expression (for source location tracking)
fn get_expr_span(expr: &Expr) -> Option<Span> {
    match expr {
        Expr::Literal(_, span)
        | Expr::Identifier(_, span)
        | Expr::Array(_, span)
        | Expr::Object(_, span)
        | Expr::Block(_, span)
        | Expr::Unit(span)
        | Expr::If(_, span)
        | Expr::While(_, span)
        | Expr::For(_, span)
        | Expr::Loop(_, span)
        | Expr::Match(_, span)
        | Expr::Let(_, span)
        | Expr::Assign(_, span)
        | Expr::TimeRef(_, span)
        | Expr::DateTime(_, span)
        | Expr::DataRef(_, span)
        | Expr::DataDateTimeRef(_, span)
        | Expr::Duration(_, span)
        | Expr::Spread(_, span)
        | Expr::ListComprehension(_, span)
        | Expr::TryOperator(_, span)
        | Expr::PatternRef(_, span)
        | Expr::WindowExpr(_, span)
        | Expr::FromQuery(_, span)
        | Expr::StructLiteral { span, .. } => Some(*span),

        Expr::BinaryOp { span, .. }
        | Expr::UnaryOp { span, .. }
        | Expr::FunctionCall { span, .. }
        | Expr::MethodCall { span, .. }
        | Expr::PropertyAccess { span, .. }
        | Expr::IndexAccess { span, .. }
        | Expr::Conditional { span, .. }
        | Expr::FuzzyComparison { span, .. }
        | Expr::EnumConstructor { span, .. }
        | Expr::TypeAssertion { span, .. }
        | Expr::InstanceOf { span, .. }
        | Expr::Range { span, .. }
        | Expr::DataRelativeAccess { span, .. }
        | Expr::TimeframeContext { span, .. }
        | Expr::SimulationCall { span, .. }
        | Expr::FunctionExpr { span, .. } => Some(*span),

        Expr::Break(_, span)
        | Expr::Continue(span)
        | Expr::Return(_, span)
        | Expr::Await(_, span)
        | Expr::Join(_, span)
        | Expr::Annotated { span, .. }
        | Expr::UsingImpl { span, .. }
        | Expr::AsyncLet(_, span)
        | Expr::AsyncScope(_, span)
        | Expr::Comptime(_, span)
        | Expr::ComptimeFor(_, span)
        | Expr::Reference { span, .. } => Some(*span),
    }
}

// Sub-modules organized by expression category
mod advanced;
mod assignment;
mod binary_ops;
mod closures;
mod collections;
mod conditionals;
mod control_flow;
mod data_access;
pub(crate) mod function_calls;
mod identifiers;
mod literals;
mod matrix_ops;
mod misc;
mod numeric_ops;
mod patterns;
mod property_access;
mod temporal;
mod type_ops;
mod unary_ops;

impl BytecodeCompiler {
    fn annotation_target_kind_for_expr(
        target: &Expr,
        forced_kind: Option<shape_ast::ast::functions::AnnotationTargetKind>,
    ) -> shape_ast::ast::functions::AnnotationTargetKind {
        if let Some(kind) = forced_kind {
            return kind;
        }
        match target {
            Expr::Annotated {
                target: inner_target,
                ..
            } => Self::annotation_target_kind_for_expr(inner_target, None),
            Expr::Block(..) => shape_ast::ast::functions::AnnotationTargetKind::Block,
            Expr::Let(..) => shape_ast::ast::functions::AnnotationTargetKind::Binding,
            _ => shape_ast::ast::functions::AnnotationTargetKind::Expression,
        }
    }

    fn comptime_target_kind_for_annotation(
        kind: shape_ast::ast::functions::AnnotationTargetKind,
    ) -> super::comptime_target::AnnotationTargetKind {
        match kind {
            shape_ast::ast::functions::AnnotationTargetKind::Function => {
                super::comptime_target::AnnotationTargetKind::Function
            }
            shape_ast::ast::functions::AnnotationTargetKind::Type => {
                super::comptime_target::AnnotationTargetKind::Type
            }
            shape_ast::ast::functions::AnnotationTargetKind::Module => {
                super::comptime_target::AnnotationTargetKind::Module
            }
            shape_ast::ast::functions::AnnotationTargetKind::Expression => {
                super::comptime_target::AnnotationTargetKind::Expression
            }
            shape_ast::ast::functions::AnnotationTargetKind::Block => {
                super::comptime_target::AnnotationTargetKind::Block
            }
            shape_ast::ast::functions::AnnotationTargetKind::AwaitExpr => {
                super::comptime_target::AnnotationTargetKind::AwaitExpr
            }
            shape_ast::ast::functions::AnnotationTargetKind::Binding => {
                super::comptime_target::AnnotationTargetKind::Binding
            }
        }
    }

    fn annotation_target_name(target: &Expr) -> String {
        match target {
            Expr::Identifier(name, _) => name.clone(),
            Expr::Let(let_expr, _) => let_expr
                .pattern
                .as_simple_name()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn run_comptime_annotation_handlers_for_target(
        &mut self,
        annotation: &shape_ast::ast::Annotation,
        target: &Expr,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
    ) -> Result<bool> {
        if let Some(compiled) = self
            .program
            .compiled_annotations
            .get(&annotation.name)
            .cloned()
        {
            let handlers = [
                compiled.comptime_pre_handler,
                compiled.comptime_post_handler,
            ];
            for handler in handlers.into_iter().flatten() {
                let mut target_desc = super::comptime_target::ComptimeTarget::for_expression();
                target_desc.kind = Self::comptime_target_kind_for_annotation(target_kind);
                target_desc.name = Self::annotation_target_name(target);
                target_desc.annotations = vec![annotation.name.clone()];
                let target_name = if target_desc.name.is_empty() {
                    "target".to_string()
                } else {
                    target_desc.name.clone()
                };
                let target_value = target_desc.to_nanboxed();
                let handler_span = handler.span;
                let execution = self.execute_comptime_annotation_handler(
                    annotation,
                    &handler,
                    target_value,
                    &[],
                )?;

                let removed = self
                    .process_comptime_directives(execution.directives, &target_name)
                    .map_err(|e| ShapeError::RuntimeError {
                        message: format!(
                            "Comptime handler '{}' directive processing failed: {}",
                            annotation.name, e
                        ),
                        location: Some(self.span_to_source_location(handler_span)),
                    })?;

                if removed {
                    self.emit(Instruction::simple(OpCode::PushNull));
                    self.last_expr_schema = None;
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    fn apply_before_result_contract(
        &mut self,
        before_result_local: u16,
        args_local: u16,
        ctx_local: u16,
        ctx_schema_id: u32,
    ) -> Result<()> {
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(before_result_local)),
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
            Some(Operand::Local(before_result_local)),
        ));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(args_local)),
        ));
        let skip_obj_check = self.emit_jump(OpCode::Jump, 0);
        self.patch_jump(skip_array);

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(before_result_local)),
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

        let before_contract_schema_id = self.type_tracker.register_inline_object_schema_typed(&[
            ("args", FieldType::Any),
            ("state", FieldType::Any),
        ]);
        let (args_operand, state_operand) = {
            let schema = self
                .type_tracker
                .schema_registry()
                .get_by_id(before_contract_schema_id)
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: "Internal error: missing before-handler schema".to_string(),
                    location: None,
                })?;
            let args_field = schema
                .get_field("args")
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: "Internal error: before-handler schema missing 'args'".to_string(),
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
            if args_field.offset > u16::MAX as usize || state_field.offset > u16::MAX as usize {
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
            )
        };

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(before_result_local)),
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
            Some(Operand::Local(before_result_local)),
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
        Ok(())
    }

    fn compile_annotated_expr(
        &mut self,
        annotation: &shape_ast::ast::Annotation,
        target: &Expr,
        ann_span: Span,
        forced_kind: Option<shape_ast::ast::functions::AnnotationTargetKind>,
    ) -> Result<()> {
        let target_kind = Self::annotation_target_kind_for_expr(target, forced_kind);
        self.validate_annotation_target_usage(annotation, target_kind, ann_span)?;
        if self.run_comptime_annotation_handlers_for_target(annotation, target, target_kind)? {
            return Ok(());
        }

        if let Some(compiled) = self
            .program
            .compiled_annotations
            .get(&annotation.name)
            .cloned()
        {
            if compiled.before_handler.is_some() || compiled.after_handler.is_some() {
                self.push_scope();
                let args_local = self.declare_local("__ann_args")?;
                let ctx_local = self.declare_local("__ann_ctx")?;
                let result_local = self.declare_local("__ann_result")?;

                // Build args array for expression annotations.
                self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(args_local)),
                ));

                // Build ctx object: { state: {}, event_log: [] }.
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
                    ("state", FieldType::Any),
                    ("event_log", FieldType::Array(Box::new(FieldType::Any))),
                ]);
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

                if let Some(before_id) = compiled.before_handler {
                    let self_ref = self.program.add_constant(Constant::Number(0.0));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(self_ref)),
                    ));
                    for ann_arg in &annotation.args {
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
                    let before_arg_count = 1 + annotation.args.len() + 2;
                    let count_const = self
                        .program
                        .add_constant(Constant::Number(before_arg_count as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(count_const)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::Call,
                        Some(Operand::Function(shape_value::FunctionId(before_id))),
                    ));
                    self.record_blob_call(before_id);

                    let before_result_local = self.declare_local("__ann_before_result")?;
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(before_result_local)),
                    ));
                    self.apply_before_result_contract(
                        before_result_local,
                        args_local,
                        ctx_local,
                        ctx_schema_id,
                    )?;
                }

                if let Expr::Annotated {
                    annotation: inner_annotation,
                    target: inner_target,
                    span: inner_span,
                } = target
                {
                    self.compile_annotated_expr(
                        inner_annotation,
                        inner_target,
                        *inner_span,
                        forced_kind,
                    )?;
                } else {
                    self.compile_expr(target)?;
                }
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));

                if let Some(after_id) = compiled.after_handler {
                    let self_ref = self.program.add_constant(Constant::Number(0.0));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(self_ref)),
                    ));
                    for ann_arg in &annotation.args {
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
                    let after_arg_count = 1 + annotation.args.len() + 3;
                    let count_const = self
                        .program
                        .add_constant(Constant::Number(after_arg_count as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(count_const)),
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

                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
                self.pop_scope();
                return Ok(());
            }
        }

        if let Expr::Annotated {
            annotation: inner_annotation,
            target: inner_target,
            span: inner_span,
        } = target
        {
            self.compile_annotated_expr(inner_annotation, inner_target, *inner_span, forced_kind)
        } else {
            self.compile_expr(target)
        }
    }

    fn compile_annotated_await_expr(
        &mut self,
        annotation: &shape_ast::ast::Annotation,
        target: &Expr,
        ann_span: Span,
    ) -> Result<()> {
        let target_kind = shape_ast::ast::functions::AnnotationTargetKind::AwaitExpr;
        self.validate_annotation_target_usage(annotation, target_kind, ann_span)?;

        if self.run_comptime_annotation_handlers_for_target(annotation, target, target_kind)? {
            return Ok(());
        }

        if let Some(compiled) = self
            .program
            .compiled_annotations
            .get(&annotation.name)
            .cloned()
        {
            if compiled.before_handler.is_some() || compiled.after_handler.is_some() {
                self.push_scope();
                let args_local = self.declare_local("__ann_args")?;
                let ctx_local = self.declare_local("__ann_ctx")?;
                let subject_local = self.declare_local("__ann_subject")?;
                let result_local = self.declare_local("__ann_result")?;

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
                    ("state", FieldType::Any),
                    ("event_log", FieldType::Array(Box::new(FieldType::Any))),
                ]);
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

                if let Expr::Annotated {
                    annotation: inner_annotation,
                    target: inner_target,
                    span: inner_span,
                } = target
                {
                    self.compile_annotated_await_expr(inner_annotation, inner_target, *inner_span)?;
                } else {
                    self.compile_expr(target)?;
                }
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(subject_local)),
                ));

                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(subject_local)),
                ));
                self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(1))));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(args_local)),
                ));

                if let Some(before_id) = compiled.before_handler {
                    let self_ref = self.program.add_constant(Constant::Number(0.0));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(self_ref)),
                    ));
                    for ann_arg in &annotation.args {
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
                    let before_arg_count = 1 + annotation.args.len() + 2;
                    let count_const = self
                        .program
                        .add_constant(Constant::Number(before_arg_count as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(count_const)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::Call,
                        Some(Operand::Function(shape_value::FunctionId(before_id))),
                    ));
                    self.record_blob_call(before_id);

                    let before_result_local = self.declare_local("__ann_before_result")?;
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(before_result_local)),
                    ));
                    self.apply_before_result_contract(
                        before_result_local,
                        args_local,
                        ctx_local,
                        ctx_schema_id,
                    )?;
                }

                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(args_local)),
                ));
                let zero_const = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(zero_const)),
                ));
                self.emit(Instruction::simple(OpCode::GetProp));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(subject_local)),
                ));

                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(subject_local)),
                ));
                self.emit(Instruction::simple(OpCode::Await));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));

                if let Some(after_id) = compiled.after_handler {
                    let self_ref = self.program.add_constant(Constant::Number(0.0));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(self_ref)),
                    ));
                    for ann_arg in &annotation.args {
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
                    let after_arg_count = 1 + annotation.args.len() + 3;
                    let count_const = self
                        .program
                        .add_constant(Constant::Number(after_arg_count as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(count_const)),
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

                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
                self.pop_scope();
                return Ok(());
            }
        }

        if let Expr::Annotated {
            annotation: inner_annotation,
            target: inner_target,
            span: inner_span,
        } = target
        {
            self.compile_annotated_await_expr(inner_annotation, inner_target, *inner_span)?;
        } else {
            self.compile_expr(target)?;
        }
        self.emit(Instruction::simple(OpCode::Await));
        Ok(())
    }

    /// Main expression compilation dispatcher
    ///
    /// This method dispatches to specialized compilation methods based on expression type.
    pub(super) fn compile_expr(&mut self, expr: &Expr) -> Result<()> {
        // Reset numeric type tracking — each expression must explicitly set it.
        // Without this, a stale numeric type from a previous sub-expression
        // could cause the wrong typed opcode to be emitted.
        self.last_expr_schema = None;
        self.last_expr_numeric_type = None;
        self.last_expr_type_info = None;

        // Track source line from expression span for error messages
        if let Some(span) = get_expr_span(expr) {
            self.set_line_from_span(span);
        }

        match expr {
            // Literals
            Expr::Literal(lit, _) => self.compile_expr_literal(lit),

            // Identifiers
            Expr::Identifier(name, span) => self.compile_expr_identifier(name, *span),

            // Binary operations
            Expr::BinaryOp {
                left, op, right, ..
            } => self.compile_expr_binary_op(left, op, right),

            // Fuzzy comparison (compile left and right, then apply fuzzy comparison)
            Expr::FuzzyComparison {
                left,
                op,
                right,
                tolerance,
                ..
            } => self.compile_expr_fuzzy_comparison(left, op, right, tolerance),

            // Unary operations
            Expr::UnaryOp { op, operand, .. } => self.compile_expr_unary_op(op, operand),

            // Type operations
            Expr::TypeAssertion {
                expr,
                type_annotation,
                ..
            } => self.compile_expr_type_assertion(expr, type_annotation),
            Expr::InstanceOf {
                expr,
                type_annotation,
                ..
            } => self.compile_expr_instanceof(expr, type_annotation),

            // Collections
            Expr::Array(elements, _) => self.compile_expr_array(elements),
            Expr::Object(fields, _) => self.compile_expr_object(fields),

            // Property and index access
            Expr::PropertyAccess {
                object,
                property,
                optional,
                ..
            } => self.compile_expr_property_access(object, property, *optional),
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => self.compile_expr_index_access(object, index, end_index),

            // Function calls
            Expr::FunctionCall {
                name, args, span, ..
            } => self.compile_expr_function_call(name, args, *span),
            Expr::MethodCall {
                receiver,
                method,
                args,
                ..
            } => self.compile_expr_method_call(receiver, method, args),
            Expr::EnumConstructor {
                enum_name,
                variant,
                payload,
                ..
            } => self.compile_expr_enum_constructor(enum_name, variant, payload),

            // Closures
            Expr::FunctionExpr { params, body, .. } => self.compile_expr_closure(params, body),

            // Conditionals
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => self.compile_expr_conditional(condition, then_expr, else_expr),
            Expr::If(if_expr, _) => self.compile_expr_if(if_expr),

            // Loops
            Expr::While(while_expr, _) => self.compile_expr_while(while_expr),
            Expr::For(for_expr, _) => self.compile_expr_for(for_expr),
            Expr::Loop(loop_expr, _) => self.compile_expr_loop(loop_expr),

            // Data access
            Expr::DataRef(data_ref, _) => self.compile_expr_data_ref(data_ref),
            Expr::DataDateTimeRef(datetime_ref, _) => {
                self.compile_expr_data_datetime_ref(datetime_ref)
            }
            Expr::DataRelativeAccess {
                reference, index, ..
            } => self.compile_expr_data_relative_access(reference, index),

            // Temporal
            Expr::TimeRef(time_ref, _) => self.compile_expr_time_ref(time_ref),
            Expr::DateTime(datetime_expr, _) => self.compile_expr_datetime(datetime_expr),
            Expr::Duration(duration, _) => self.compile_expr_duration(duration),
            Expr::TimeframeContext {
                timeframe, expr, ..
            } => self.compile_expr_timeframe_context(*timeframe, expr),

            // Control flow
            Expr::Break(value_expr, _) => self.compile_expr_break(value_expr),
            Expr::Continue(_) => self.compile_expr_continue(),
            Expr::Return(value_expr, _) => self.compile_expr_return(value_expr),

            // Let and assignment
            Expr::Let(let_expr, _) => self.compile_expr_let(let_expr),
            Expr::Assign(assign_expr, _) => self.compile_expr_assign(assign_expr),

            // Advanced expressions
            Expr::ListComprehension(comp, _) => self.compile_expr_list_comprehension(comp),
            Expr::TryOperator(inner, _) => self.compile_expr_try_operator(inner),
            Expr::UsingImpl {
                expr, impl_name, ..
            } => self.compile_expr_using_impl(expr, impl_name),
            Expr::Match(match_expr, _) => self.compile_expr_match(match_expr),

            // Pattern references
            Expr::PatternRef(name, _) => self.compile_expr_pattern_ref(name),

            // Miscellaneous
            Expr::Unit(_) => self.compile_expr_unit(),
            Expr::Spread(..) => self.compile_expr_spread(),
            Expr::Block(block, _) => self.compile_expr_block(block),
            Expr::Range {
                start, end, kind, ..
            } => self.compile_expr_range(start, end, kind),

            Expr::WindowExpr(window_expr, _) => self.compile_expr_window(window_expr),
            Expr::SimulationCall { .. } => Err(shape_ast::error::ShapeError::RuntimeError {
                message: "Simulation calls not supported".to_string(),
                location: None,
            }),

            // FromQuery should have been desugared before compilation
            Expr::FromQuery(_, _) => Err(shape_ast::error::ShapeError::RuntimeError {
                message: "FromQuery expressions must be desugared before compilation".to_string(),
                location: None,
            }),

            // Struct literal: TypeName { field: value, ... }
            Expr::StructLiteral {
                type_name,
                fields,
                span,
            } => self.compile_struct_literal(type_name, fields, *span),

            // Await expression: compile inner expr, emit Await opcode
            Expr::Await(inner, _span) => {
                if self.current_function.is_some() && !self.current_function_is_async {
                    return Err(shape_ast::error::ShapeError::SemanticError {
                        message: "'await' can only be used inside an async function".to_string(),
                        location: None,
                    });
                }
                if let Expr::Annotated {
                    annotation,
                    target,
                    span,
                } = inner.as_ref()
                {
                    self.compile_annotated_await_expr(annotation, target, *span)?;
                } else {
                    self.compile_expr(inner)?;
                    self.emit(Instruction::simple(OpCode::Await));
                }
                Ok(())
            }

            // Join expression: await join all|race|any|settle { branch1, branch2, ... }
            // Note: Expr::Join is always wrapped in Expr::Await by the parser
            Expr::Join(join_expr, _span) => self.compile_join_expr(join_expr),

            // Annotated expression: @annotation expr
            Expr::Annotated {
                annotation,
                target,
                span,
            } => self.compile_annotated_expr(annotation, target, *span, None),

            // Async let: spawn task and bind future to local variable
            Expr::AsyncLet(async_let, _) => self.compile_async_let(async_let),

            // Async scope: structured concurrency boundary
            Expr::AsyncScope(inner, _) => self.compile_async_scope(inner),

            // Comptime blocks: execute at compile time, emit result as a constant
            Expr::Comptime(stmts, span) => {
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
                .map_err(|e| shape_ast::error::ShapeError::RuntimeError {
                    message: format!(
                        "Comptime block evaluation failed: {}",
                        super::helpers::strip_error_prefix(&e)
                    ),
                    location: Some(self.span_to_source_location(*span)),
                })?;
                // Comptime blocks can emit directives via direct syntax.
                // They are processed with no implicit target binding.
                self.process_comptime_directives(execution.directives, "")
                    .map_err(|e| shape_ast::error::ShapeError::RuntimeError {
                        message: format!("Comptime block directive processing failed: {}", e),
                        location: Some(self.span_to_source_location(*span)),
                    })?;
                // Convert the result to a literal and compile it
                let lit = super::comptime::vmvalue_to_literal(&execution.value);
                self.compile_literal(&lit)?;
                self.last_expr_schema = None;
                Ok(())
            }

            // Comptime for: evaluate iterable at compile time, unroll body for each element.
            Expr::ComptimeFor(cf, span) => self.compile_comptime_for(cf, *span),

            // Reference expression (&var / &mut var) - create a reference to a local variable.
            // Valid both as function arguments and standalone expressions (e.g., `let r = &x`).
            Expr::Reference {
                expr: inner,
                is_mutable,
                span,
            } => {
                let mode = if self.in_call_args {
                    self.current_arg_borrow_mode()
                } else if *is_mutable {
                    BorrowMode::Exclusive
                } else {
                    BorrowMode::Shared
                };
                match inner.as_ref() {
                    Expr::Identifier(name, id_span) => {
                        self.compile_reference_identifier(name, *id_span, mode)
                    }
                    _ => Err(ShapeError::SemanticError {
                        message: "`&` can only be applied to a simple variable name (e.g., `&x`), not a complex expression".to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    }),
                }
            }
        }
    }

    /// Infer the type of an expression using the type inference engine
    ///
    /// Used for match exhaustiveness checking and other type-based validations.
    pub(super) fn infer_expr_type(
        &mut self,
        expr: &Expr,
    ) -> Result<shape_runtime::type_system::Type> {
        self.type_inference.infer_expr(expr).map_err(|e| {
            shape_ast::error::ShapeError::SemanticError {
                message: format!("Type inference failed: {:?}", e),
                location: None,
            }
        })
    }
}
