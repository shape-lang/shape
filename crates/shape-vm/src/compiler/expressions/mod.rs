//! Expression compilation
//!
//! This module contains the main expression compilation logic, organized by expression type.

use shape_ast::ast::{Expr, Span};
use shape_ast::error::{Result, ShapeError};

use super::{BorrowMode, BytecodeCompiler, ExprReferenceResult, ExprResultMode};
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
        | Expr::QualifiedFunctionCall { span, .. }
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
        | Expr::Reference { span, .. }
        | Expr::TableRows(_, span) => Some(*span),
    }
}

// Sub-modules organized by expression category
mod advanced;
mod assignment;
mod binary_ops;
pub(crate) mod closures;
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
        if let Some((_, compiled)) = self.lookup_compiled_annotation(annotation) {
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
                    &compiled.param_names,
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

    /// Apply the before-handler result contract.
    ///
    /// The before handler can return:
    /// - An array → replaces args
    /// - An object `{ args?, state?, result? }` → updates args/state, and if
    ///   `result` is non-null, short-circuits (skips impl call / expression eval)
    ///
    /// When `result_local` is `Some`, the `result` field is extracted and stored
    /// there, and a short-circuit jump is emitted. The returned `Option<usize>`
    /// is the jump address that must be patched by the caller to skip past the
    /// impl call / expression evaluation.
    fn apply_before_result_contract(
        &mut self,
        before_result_local: u16,
        args_local: u16,
        ctx_local: u16,
        ctx_schema_id: u32,
    ) -> Result<()> {
        self.apply_before_result_contract_inner(
            before_result_local,
            args_local,
            ctx_local,
            ctx_schema_id,
            None,
        )
        .map(|_| ())
    }

    /// Like `apply_before_result_contract` but with short-circuit support.
    ///
    /// When `short_circuit_result_local` is provided, the `result` field of the
    /// before-handler object is extracted. If non-null, the value is stored in
    /// the given local and a jump is emitted. The returned `Option<usize>` is
    /// the jump that must be patched to skip past the impl/expression.
    fn apply_before_result_contract_with_short_circuit(
        &mut self,
        before_result_local: u16,
        args_local: u16,
        ctx_local: u16,
        ctx_schema_id: u32,
        short_circuit_result_local: u16,
    ) -> Result<Option<usize>> {
        self.apply_before_result_contract_inner(
            before_result_local,
            args_local,
            ctx_local,
            ctx_schema_id,
            Some(short_circuit_result_local),
        )
    }

    fn apply_before_result_contract_inner(
        &mut self,
        before_result_local: u16,
        args_local: u16,
        ctx_local: u16,
        ctx_schema_id: u32,
        short_circuit_result_local: Option<u16>,
    ) -> Result<Option<usize>> {
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(before_result_local)),
        ));
        let one_const = self.program.add_constant(Constant::Int(1));
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
        let one_const2 = self.program.add_constant(Constant::Int(1));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(one_const2)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(crate::bytecode::BuiltinFunction::IsObject)),
        ));
        let skip_obj = self.emit_jump(OpCode::JumpIfFalse, 0);

        // Schema includes `result` field for short-circuit support
        let before_contract_schema_id = self.type_tracker.register_inline_object_schema_typed(&[
            ("args", FieldType::Any),
            ("result", FieldType::Any),
            ("state", FieldType::Any),
        ]);
        let (args_operand, state_operand, result_operand) = {
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

        // Check `result` field for short-circuit
        let mut short_circuit_jump = None;
        if let Some(sc_local) = short_circuit_result_local {
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result_local)),
            ));
            self.emit(Instruction::new(
                OpCode::GetFieldTyped,
                Some(result_operand),
            ));
            // Stage 2.6.5.2: typed IsNull replaces `PushNull; Eq`.
            self.emit(Instruction::simple(OpCode::Dup));
            self.emit(Instruction::simple(OpCode::IsNull));
            let skip_short_circuit = self.emit_jump(OpCode::JumpIfTrue, 0);
            // result is non-null → store it and jump past impl
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(sc_local)),
            ));
            short_circuit_jump = Some(self.emit_jump(OpCode::Jump, 0));
            self.patch_jump(skip_short_circuit);
            self.emit(Instruction::simple(OpCode::Pop)); // discard null result
        }

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(before_result_local)),
        ));
        self.emit(Instruction::new(OpCode::GetFieldTyped, Some(args_operand)));
        // Stage 2.6.5.2: typed IsNull replaces `PushNull; Eq`.
        self.emit(Instruction::simple(OpCode::Dup));
        self.emit(Instruction::simple(OpCode::IsNull));
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
        // Stage 2.6.5.2: typed IsNull replaces `PushNull; Eq`.
        self.emit(Instruction::simple(OpCode::Dup));
        self.emit(Instruction::simple(OpCode::IsNull));
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
        Ok(short_circuit_jump)
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
                        .add_constant(Constant::Int(before_arg_count as i64));
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
                        .add_constant(Constant::Int(after_arg_count as i64));
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

                // Initialize args as empty array (before handler gets annotation
                // args + ctx, not the evaluated expression)
                self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(args_local)),
                ));

                // Call before handler FIRST (before evaluating inner expression).
                // This allows short-circuit: if before returns { result: value },
                // we skip the inner expression eval + await entirely.
                let mut short_circuit_jump = None;
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
                        .add_constant(Constant::Int(before_arg_count as i64));
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
                    short_circuit_jump = self.apply_before_result_contract_with_short_circuit(
                        before_result_local,
                        args_local,
                        ctx_local,
                        ctx_schema_id,
                        result_local,
                    )?;
                }

                // --- Normal path: evaluate inner expression + await ---
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
                self.emit(Instruction::simple(OpCode::Await));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));

                // Patch the short-circuit jump to land here (after await, at result usage)
                if let Some(jump_addr) = short_circuit_jump {
                    self.patch_jump(jump_addr);
                }

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
                        .add_constant(Constant::Int(after_arg_count as i64));
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

    pub(super) fn capture_last_expr_reference_result(&self) -> ExprReferenceResult {
        self.last_expr_reference_result
    }

    pub(super) fn restore_last_expr_reference_result(&mut self, result: ExprReferenceResult) {
        self.last_expr_reference_result = result;
    }

    pub(super) fn clear_last_expr_reference_result(&mut self) {
        self.last_expr_reference_result = ExprReferenceResult::default();
    }

    pub(super) fn set_last_expr_reference_result(&mut self, mode: BorrowMode, auto_deref: bool) {
        self.last_expr_reference_result = ExprReferenceResult {
            raw_mode: Some(mode),
            auto_deref_mode: auto_deref.then_some(mode),
        };
    }

    pub(super) fn last_expr_reference_mode(&self) -> Option<BorrowMode> {
        self.last_expr_reference_result.raw_mode
    }

    pub(super) fn merge_reference_results(results: &[ExprReferenceResult]) -> ExprReferenceResult {
        let Some(first) = results.first().copied() else {
            return ExprReferenceResult::default();
        };
        let Some(raw_mode) = first.raw_mode else {
            return ExprReferenceResult::default();
        };
        if !results
            .iter()
            .all(|result| result.raw_mode == Some(raw_mode))
        {
            return ExprReferenceResult::default();
        }
        let auto_deref_mode = if first.auto_deref_mode.is_some()
            && results
                .iter()
                .all(|result| result.auto_deref_mode == first.auto_deref_mode)
        {
            first.auto_deref_mode
        } else {
            None
        };
        ExprReferenceResult {
            raw_mode: Some(raw_mode),
            auto_deref_mode,
        }
    }

    fn auto_deref_last_expr_result_if_needed(&mut self) -> Result<()> {
        if self.last_expr_reference_result.auto_deref_mode.is_none() {
            return Ok(());
        }
        let temp = self.declare_temp_local("__expr_auto_deref_")?;
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(temp)),
        ));
        self.emit(Instruction::new(
            OpCode::DerefLoad,
            Some(Operand::Local(temp)),
        ));
        self.clear_last_expr_reference_result();
        Ok(())
    }

    pub(super) fn current_expr_result_mode(&self) -> ExprResultMode {
        self.current_expr_result_mode
    }

    pub(super) fn compile_expr_preserving_refs(&mut self, expr: &Expr) -> Result<()> {
        let saved_mode = self.current_expr_result_mode;
        self.current_expr_result_mode = ExprResultMode::PreserveRef;
        self.clear_last_expr_reference_result();

        let result = match expr {
            Expr::Identifier(name, span) => {
                self.compile_expr_identifier_preserving_refs(name, *span)
            }
            Expr::FunctionCall {
                name, args, span, ..
            } => self.compile_expr_function_call(name, args, *span),
            Expr::QualifiedFunctionCall {
                namespace,
                function,
                args,
                span,
                ..
            } => self.compile_expr_qualified_function_call(namespace, function, args, *span),
            Expr::MethodCall {
                receiver,
                method,
                args,
                span,
                ..
            } => self.compile_expr_method_call(receiver, method, args, *span),
            Expr::Reference {
                expr: inner,
                is_mutable,
                span,
            } => {
                let mode = if *is_mutable {
                    BorrowMode::Exclusive
                } else {
                    BorrowMode::Shared
                };
                let result = self.compile_reference_expr(inner, *span, mode).map(|_| ());
                if result.is_ok() {
                    self.set_last_expr_reference_result(mode, false);
                }
                result
            }
            Expr::Block(block, _) => self.compile_expr_block(block),
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => self.compile_expr_conditional(condition, then_expr, else_expr),
            Expr::If(if_expr, _) => self.compile_expr_if(if_expr),
            Expr::Let(let_expr, _) => self.compile_expr_let(let_expr),
            Expr::Assign(assign_expr, _) => self.compile_expr_assign(assign_expr),
            Expr::Match(match_expr, _) => self.compile_expr_match(match_expr),
            _ => {
                let result = self.compile_expr(expr);
                if result.is_ok() {
                    self.clear_last_expr_reference_result();
                }
                result
            }
        };

        self.current_expr_result_mode = saved_mode;
        result
    }

    /// Main expression compilation dispatcher
    ///
    /// This method dispatches to specialized compilation methods based on expression type.
    pub(super) fn compile_expr(&mut self, expr: &Expr) -> Result<()> {
        let saved_mode = self.current_expr_result_mode;
        self.current_expr_result_mode = ExprResultMode::Value;
        self.clear_last_expr_reference_result();

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

        let result = match expr {
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
            Expr::QualifiedFunctionCall {
                namespace,
                function,
                args,
                span,
                ..
            } => self.compile_expr_qualified_function_call(namespace, function, args, *span),
            Expr::MethodCall {
                receiver,
                method,
                args,
                span,
                ..
            } => self.compile_expr_method_call(receiver, method, args, *span),
            Expr::EnumConstructor {
                enum_name,
                variant,
                payload,
                span,
                ..
            } => {
                // Check if this is a Type::comptime_field access (looks like enum syntax)
                if matches!(payload, shape_ast::ast::EnumConstructorPayload::Unit) {
                    if self
                        .comptime_fields
                        .get(enum_name.as_str())
                        .and_then(|m| m.get(variant))
                        .is_some()
                    {
                        // SURFACE: the kinded `KindedSlot → Constant`
                        // projection used by comptime field extraction
                        // (treated here as the enum-constructor-shaped
                        // dotted path `Currency.symbol` → `Currency::symbol`)
                        // lives in phase-2c (ADR-006 §2.4 / §2.7.4). The
                        // producer side that populates `comptime_fields`
                        // (`statements.rs:2450-2512`) is dormant, so this
                        // branch is currently unreachable in real
                        // programs. Tracked as `c3-expr-lowering-misc`
                        // per playbook §3.
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "comptime field access '{}.{}' (via enum-constructor path) \
                                 is dormant pending the phase-2c KindedSlot-to-Constant \
                                 projection rebuild (ADR-006 §2.4 / §2.7.4)",
                                enum_name, variant
                            ),
                            location: Some(self.span_to_source_location(*span)),
                        });
                    }
                }
                self.compile_expr_enum_constructor(enum_name, variant, payload)
            }

            // Closures
            Expr::FunctionExpr {
                params, body, span, ..
            } => self.compile_expr_closure(params, body, *span),

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
                // W7 (2026-05-17): TypeReflectionSnapshot for `type_info(T)`
                // resolution from a comptime expression.
                let type_snapshot = super::comptime_builtins::build_type_reflection_snapshot(
                    self,
                    &[],
                );
                let execution = super::comptime::execute_comptime(
                    stmts,
                    &comptime_helpers,
                    &extensions,
                    trait_impls,
                    known_type_symbols,
                    type_snapshot,
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
                // Convert the result to an expression and compile it.
                // Use nb_to_expr for complex types (arrays, objects) that
                // cannot be represented as a single literal.
                if let Ok(expr) = super::comptime::nb_to_expr_public(&execution.value, *span) {
                    self.compile_expr(&expr)?;
                } else {
                    let lit = super::comptime::vmvalue_to_literal(&execution.value);
                    self.compile_literal(&lit)?;
                }
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
                let mode = if *is_mutable {
                    BorrowMode::Exclusive
                } else {
                    BorrowMode::Shared
                };
                let result = self.compile_reference_expr(inner, *span, mode).map(|_| ());
                if result.is_ok() {
                    self.set_last_expr_reference_result(mode, false);
                }
                result
            }

            // Table row literals — compiled via compile_table_rows() in the VariableDecl handler.
            // If we reach here, it means TableRows appeared outside a let binding context.
            Expr::TableRows(_, span) => Err(ShapeError::SemanticError {
                message: "table row literal `[...], [...]` can only be used as a variable initializer with a `Table<T>` type annotation".to_string(),
                location: Some(self.span_to_source_location(*span)),
            }),
        };

        if result.is_ok() {
            self.auto_deref_last_expr_result_if_needed()?;
        }
        self.current_expr_result_mode = saved_mode;
        result
    }

    /// Infer the type of an expression using the type inference engine
    ///
    /// Used for match exhaustiveness checking and other type-based validations.
    ///
    /// R5.3B: for `Expr::Identifier`, the compiler-owned `type_tracker`
    /// holds the authoritative type_name for let-locals, typed function
    /// parameters, and module bindings. The `type_inference` engine does
    /// not define those bindings in its environment, so it returns
    /// `UndefinedVariable` for the same identifiers. Consulting the tracker
    /// first preserves the temporal display name
    /// (`"DateTime"` / `"Duration"` / `"TimeSpan"`) through identifier
    /// resolution so the retarget guards at `binary_ops.rs:750-771` (Add)
    /// and `:1049-1072` (Sub) fire uniformly. For non-temporal identifiers
    /// the tracker value is equally valid (it matches the declared or
    /// inferred type), but we scope the tracker short-circuit narrowly to
    /// temporal names to avoid changing any existing non-temporal
    /// `infer_expr_type` behavior.
    pub(super) fn infer_expr_type(
        &mut self,
        expr: &Expr,
    ) -> Result<shape_runtime::type_system::Type> {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::Type;

        if let Expr::Identifier(name, _) = expr {
            if let Some(type_name) = self.tracker_type_name_for_identifier(name) {
                if matches!(
                    type_name.as_str(),
                    "DateTime" | "Duration" | "TimeSpan"
                ) {
                    return Ok(Type::Concrete(TypeAnnotation::Basic(type_name)));
                }
                // Strict-typing-sweep: trust the type tracker for any
                // primitive scalar name. The runtime inference engine
                // ran on the original program AST and doesn't see
                // function-body `let a: u32 = 42` declarations, so
                // identifier inference returns Variable for those. The
                // tracker, in contrast, sees the annotation when
                // `compile_function_body` propagates declared types
                // into local slots. Falling back to it for primitive
                // names plugs the strict-typing hole that previously
                // routed through the deleted *Dynamic* shim.
                if shape_runtime::type_system::BuiltinTypes::is_integer_type_name(&type_name)
                    || shape_runtime::type_system::BuiltinTypes::is_number_type_name(&type_name)
                    || matches!(
                        type_name.as_str(),
                        "bool" | "string" | "decimal" | "bigint"
                    )
                {
                    return Ok(Type::Concrete(TypeAnnotation::Basic(type_name)));
                }
            }
        }

        // Phase 3e: function call return type from the tracker. The
        // runtime type-inference engine doesn't always see freshly
        // declared user functions; the tracker's
        // `function_return_types` is populated by the inference
        // pre-pass (`infer_return_type_hints_from_types`) and serves as
        // the authoritative source for inferred return types in the
        // compiler's strict-typing decisions.
        if let Expr::FunctionCall { name, .. } = expr {
            if let Some(rt_name) = self
                .type_tracker
                .get_function_return_type(name)
                .cloned()
            {
                return Ok(Type::Concrete(TypeAnnotation::Basic(rt_name)));
            }
            // Sweep phase 3c.1: closure-binding return type. When the
            // call target is a `let f = |…| …` local or module binding,
            // its return type is recorded by
            // `update_callable_binding_from_expr`. Without this lookup,
            // `f(5) + f(7)` would fail strict typing as
            // `unknown + unknown`.
            if let Some(local_idx) = self.resolve_local(name) {
                if let Some(rt_name) = self
                    .local_callable_return_types
                    .get(&local_idx)
                    .cloned()
                {
                    return Ok(Type::Concrete(TypeAnnotation::Basic(rt_name)));
                }
            }
            if let Some(scoped) = self.resolve_scoped_module_binding_name(name) {
                if let Some(&binding_idx) = self.module_bindings.get(&scoped) {
                    if let Some(rt_name) = self
                        .module_binding_callable_return_types
                        .get(&binding_idx)
                        .cloned()
                    {
                        return Ok(Type::Concrete(TypeAnnotation::Basic(rt_name)));
                    }
                }
            }
            if let Some(&binding_idx) = self.module_bindings.get(name) {
                if let Some(rt_name) = self
                    .module_binding_callable_return_types
                    .get(&binding_idx)
                    .cloned()
                {
                    return Ok(Type::Concrete(TypeAnnotation::Basic(rt_name)));
                }
            }
        }

        // Sweep phase 3c.x: callable-array-element invocation. The parser
        // models `arr[i](args...)` as
        // `MethodCall { method: "__call__", receiver: IndexAccess { object: Identifier(arr), .. }, .. }`.
        // When `arr` is a `let arr = [|...| ..., ...]` binding whose elements
        // are closures with a homogeneous return type, recover that type
        // from `local_array_callable_return_types` /
        // `module_binding_array_callable_return_types` so binops like
        // `arr[0](1) + arr[1](1)` can dispatch under strict typing.
        if let Expr::MethodCall { receiver, method, .. } = expr {
            if method == "__call__" {
                if let Expr::IndexAccess { object, .. } = receiver.as_ref() {
                    if let Expr::Identifier(arr_name, _) = object.as_ref() {
                        if let Some(local_idx) = self.resolve_local(arr_name) {
                            if let Some(rt_name) = self
                                .local_array_callable_return_types
                                .get(&local_idx)
                                .cloned()
                            {
                                return Ok(Type::Concrete(TypeAnnotation::Basic(rt_name)));
                            }
                        }
                        if let Some(scoped) =
                            self.resolve_scoped_module_binding_name(arr_name)
                        {
                            if let Some(&binding_idx) =
                                self.module_bindings.get(&scoped)
                            {
                                if let Some(rt_name) = self
                                    .module_binding_array_callable_return_types
                                    .get(&binding_idx)
                                    .cloned()
                                {
                                    return Ok(Type::Concrete(TypeAnnotation::Basic(
                                        rt_name,
                                    )));
                                }
                            }
                        }
                        if let Some(&binding_idx) = self.module_bindings.get(arr_name) {
                            if let Some(rt_name) = self
                                .module_binding_array_callable_return_types
                                .get(&binding_idx)
                                .cloned()
                            {
                                return Ok(Type::Concrete(TypeAnnotation::Basic(rt_name)));
                            }
                        }
                    }
                }
            }
        }

        // Phase 3e: BinaryOp Add of string-typed operands yields a string.
        // The runtime type-inference engine doesn't know about let-mut
        // accumulator types from the tracker, so chained concats like
        // `result + name + " "` would otherwise resolve to Unknown for
        // any inner sub-expression that isn't a bare identifier.
        if let Expr::BinaryOp { op: shape_ast::ast::BinaryOp::Add, left, right, .. } = expr {
            let lt = self.infer_expr_type(left).ok();
            let rt = self.infer_expr_type(right).ok();
            let is_string = |t: &Option<shape_runtime::type_system::Type>| {
                matches!(
                    t,
                    Some(shape_runtime::type_system::Type::Concrete(
                        TypeAnnotation::Basic(n)
                    )) if n == "string" || n == "char"
                )
            };
            if is_string(&lt) && is_string(&rt) {
                return Ok(Type::Concrete(TypeAnnotation::Basic("string".to_string())));
            }
        }

        // Phase 3d: TypedObject self-field type propagation.
        // For `expr.field`, when the receiver is an identifier with a known
        // schema in the tracker (e.g. `self` in a trait method body, or any
        // typed local), look up the field type in the schema registry and
        // map it to a concrete type annotation. This plugs a strict-typing
        // hole where `infer_expr_type` for `self.name` would fall through
        // to the runtime inference engine — which doesn't know `self`'s
        // type — and return Unknown.
        if let Expr::PropertyAccess { object, property, optional, .. } = expr {
            if !*optional {
                if let Some(schema_id) = self.tracker_schema_id_for_expr(object) {
                    if let Some(field_ty) = self
                        .type_tracker
                        .schema_registry()
                        .get_by_id(schema_id)
                        .and_then(|schema| schema.get_field(property))
                        .map(|field| field.field_type.clone())
                    {
                        if let Some(ann) = field_type_to_annotation(&field_ty) {
                            return Ok(Type::Concrete(ann));
                        }
                    }
                }
            }
        }

        self.type_inference.infer_expr(expr).map_err(|e| {
            shape_ast::error::ShapeError::SemanticError {
                message: format!("Type inference failed: {:?}", e),
                location: None,
            }
        })
    }

    /// R5.3B helper: return the tracker-recorded `type_name` for an
    /// identifier, searching local slots first and falling back to module
    /// bindings. Returns `None` if the identifier is neither a local nor a
    /// module binding, or if the tracker has no type_name on that slot.
    pub(super) fn tracker_type_name_for_identifier(&self, name: &str) -> Option<String> {
        if let Some(local_idx) = self.resolve_local(name) {
            if let Some(info) = self.type_tracker.get_local_type(local_idx) {
                if let Some(ref tn) = info.type_name {
                    return Some(tn.clone());
                }
            }
        }
        if let Some(&binding_idx) = self.module_bindings.get(name) {
            if let Some(info) = self.type_tracker.get_binding_type(binding_idx) {
                if let Some(ref tn) = info.type_name {
                    return Some(tn.clone());
                }
            }
        }
        None
    }

    /// Phase 3d helper: look up a tracker-recorded schema_id for an
    /// expression. Currently handles the identifier case (locals + module
    /// bindings), which covers the `self.field` use case in trait method
    /// bodies.
    fn tracker_schema_id_for_expr(&self, expr: &Expr) -> Option<u32> {
        let lookup_by_name = |tn: &str| -> Option<u32> {
            self.type_tracker
                .schema_registry()
                .get(tn)
                .map(|s| s.id)
                .or_else(|| {
                    // Phase 3e: fall back to module-scope-resolved name
                    // (e.g. `A` inside `mod m` resolves to `m::A`). The
                    // schema is registered under the qualified form;
                    // local/binding type_name often holds the bare form.
                    let qualified = self.resolve_type_name(tn);
                    if qualified != tn {
                        self.type_tracker
                            .schema_registry()
                            .get(&qualified)
                            .map(|s| s.id)
                    } else {
                        None
                    }
                })
        };
        if let Expr::Identifier(name, _) = expr {
            if let Some(local_idx) = self.resolve_local(name) {
                if let Some(info) = self.type_tracker.get_local_type(local_idx) {
                    if let Some(id) = info.schema_id {
                        return Some(id);
                    }
                    if let Some(ref tn) = info.type_name {
                        if let Some(id) = lookup_by_name(tn) {
                            return Some(id);
                        }
                    }
                }
            }
            if let Some(&binding_idx) = self.module_bindings.get(name) {
                if let Some(info) = self.type_tracker.get_binding_type(binding_idx) {
                    if let Some(id) = info.schema_id {
                        return Some(id);
                    }
                    if let Some(ref tn) = info.type_name {
                        if let Some(id) = lookup_by_name(tn) {
                            return Some(id);
                        }
                    }
                }
            }
        }
        None
    }
}

/// Phase 3d helper: map a `FieldType` to a `TypeAnnotation` concrete enough
/// for `infer_expr_type` consumers (string-concat path, numeric coercion,
/// etc.). Returns None when the field type doesn't have a useful primitive
/// annotation (e.g. arrays / objects / Any), in which case the caller falls
/// back to the inference engine.
fn field_type_to_annotation(
    ft: &shape_runtime::type_schema::FieldType,
) -> Option<shape_ast::ast::TypeAnnotation> {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_schema::FieldType;
    match ft {
        FieldType::String => Some(TypeAnnotation::Basic("string".to_string())),
        FieldType::I64 => Some(TypeAnnotation::Basic("int".to_string())),
        FieldType::F64 => Some(TypeAnnotation::Basic("number".to_string())),
        FieldType::Bool => Some(TypeAnnotation::Basic("bool".to_string())),
        FieldType::Decimal => Some(TypeAnnotation::Basic("decimal".to_string())),
        FieldType::Timestamp => Some(TypeAnnotation::Basic("DateTime".to_string())),
        FieldType::Object(name) => Some(TypeAnnotation::Basic(name.clone())),
        FieldType::I8 => Some(TypeAnnotation::Basic("i8".to_string())),
        FieldType::U8 => Some(TypeAnnotation::Basic("u8".to_string())),
        FieldType::I16 => Some(TypeAnnotation::Basic("i16".to_string())),
        FieldType::U16 => Some(TypeAnnotation::Basic("u16".to_string())),
        FieldType::I32 => Some(TypeAnnotation::Basic("i32".to_string())),
        FieldType::U32 => Some(TypeAnnotation::Basic("u32".to_string())),
        FieldType::U64 => Some(TypeAnnotation::Basic("u64".to_string())),
        FieldType::Array(_) | FieldType::Any => None,
    }
}
