//! Reference tracking, borrow key management, and callable pass mode utilities

use super::{BorrowMode, BorrowPlace};
use crate::bytecode::{Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::{BindingStorageClass, VariableKind, VariableTypeInfo};
use shape_ast::ast::{BlockItem, Expr, Item, Statement};
use shape_ast::error::{Result, ShapeError, SourceLocation};
use shape_runtime::type_schema::FieldType;
use std::collections::HashSet;

use super::{BytecodeCompiler, FunctionReturnReferenceSummary, ParamPassMode};

pub(super) struct TypedFieldPlace {
    pub root_name: String,
    pub is_local: bool,
    pub slot: u16,
    pub typed_operand: Operand,
    pub borrow_key: BorrowPlace,
    pub field_type_info: FieldType,
}

impl BytecodeCompiler {
    const MODULE_BINDING_BORROW_FLAG: BorrowPlace = 0x8000_0000;
    const FIELD_BORROW_SHIFT: u32 = 16;

    pub(super) fn borrow_key_for_local(local_idx: u16) -> BorrowPlace {
        local_idx as BorrowPlace
    }

    pub(super) fn borrow_key_for_module_binding(binding_idx: u16) -> BorrowPlace {
        Self::MODULE_BINDING_BORROW_FLAG | binding_idx as BorrowPlace
    }

    fn borrow_key_is_module_binding(place: BorrowPlace) -> bool {
        (place & Self::MODULE_BINDING_BORROW_FLAG) != 0
    }

    pub(super) fn check_read_allowed_in_current_context(
        &self,
        _place: BorrowPlace,
        _source_location: Option<SourceLocation>,
    ) -> Result<()> {
        Ok(()) // MIR analysis is the sole authority
    }

    fn encode_field_borrow(field_idx: u16) -> BorrowPlace {
        ((field_idx as BorrowPlace + 1) & 0x7FFF) << Self::FIELD_BORROW_SHIFT
    }

    pub(super) fn borrow_key_for_local_field(local_idx: u16, field_idx: u16) -> BorrowPlace {
        Self::borrow_key_for_local(local_idx) | Self::encode_field_borrow(field_idx)
    }

    pub(super) fn borrow_key_for_module_binding_field(
        binding_idx: u16,
        field_idx: u16,
    ) -> BorrowPlace {
        Self::borrow_key_for_module_binding(binding_idx) | Self::encode_field_borrow(field_idx)
    }

    pub(super) fn relabel_borrow_error(
        err: ShapeError,
        borrow_key: BorrowPlace,
        label: &str,
    ) -> ShapeError {
        match err {
            ShapeError::SemanticError { message, location } => ShapeError::SemanticError {
                message: message
                    .replace(&format!("(slot {})", borrow_key), &format!("'{}'", label)),
                location,
            },
            other => other,
        }
    }

    pub(super) fn try_resolve_typed_field_place(
        &self,
        object: &Expr,
        property: &str,
    ) -> Option<TypedFieldPlace> {
        let (root_name, is_local, slot, type_info) = match object {
            Expr::Identifier(name, _) => {
                if let Some(local_idx) = self.resolve_local(name) {
                    if self.ref_locals.contains(&local_idx)
                        || self.reference_value_locals.contains(&local_idx)
                    {
                        return None;
                    }
                    (
                        name.clone(),
                        true,
                        local_idx,
                        self.type_tracker.get_local_type(local_idx)?.clone(),
                    )
                } else {
                    let scoped_name = self.resolve_scoped_module_binding_name(name)?;
                    let binding_idx = *self.module_bindings.get(&scoped_name)?;
                    if self.reference_value_module_bindings.contains(&binding_idx) {
                        return None;
                    }
                    (
                        name.clone(),
                        false,
                        binding_idx,
                        self.type_tracker.get_binding_type(binding_idx)?.clone(),
                    )
                }
            }
            // Recursive case: handle chained property access like `a.b.c`
            Expr::PropertyAccess {
                object: inner_object,
                property: inner_property,
                optional: false,
                ..
            } => {
                // Resolve the intermediate field place first
                let parent = self.try_resolve_typed_field_place(inner_object, inner_property)?;
                // The intermediate field must be a nested Object type with a known schema
                let nested_type_name = match &parent.field_type_info {
                    FieldType::Object(name) => name.clone(),
                    _ => return None,
                };
                let nested_schema = self.type_tracker.schema_registry().get(&nested_type_name)?;
                let nested_field = nested_schema.get_field(property)?;
                let nested_field_idx = nested_field.index as u16;
                // For chained borrows, the borrow key uses the root slot + leaf field
                let borrow_key = if parent.is_local {
                    Self::borrow_key_for_local_field(parent.slot, nested_field_idx)
                } else {
                    Self::borrow_key_for_module_binding_field(parent.slot, nested_field_idx)
                };
                return Some(TypedFieldPlace {
                    root_name: parent.root_name,
                    is_local: parent.is_local,
                    slot: parent.slot,
                    typed_operand: Operand::TypedField {
                        type_id: nested_schema.id as u16,
                        field_idx: nested_field_idx,
                        field_type_tag: field_type_to_tag(&nested_field.field_type),
                    },
                    borrow_key,
                    field_type_info: nested_field.field_type.clone(),
                });
            }
            _ => return None,
        };

        if !matches!(type_info.kind, VariableKind::Value) {
            return None;
        }

        let schema_id = type_info.schema_id?;
        if schema_id > u16::MAX as u32 {
            return None;
        }

        let schema = self.type_tracker.schema_registry().get_by_id(schema_id)?;
        let field = schema.get_field(property)?;
        let field_idx = field.index as u16;
        let borrow_key = if is_local {
            Self::borrow_key_for_local_field(slot, field_idx)
        } else {
            Self::borrow_key_for_module_binding_field(slot, field_idx)
        };

        Some(TypedFieldPlace {
            root_name,
            is_local,
            slot,
            typed_operand: Operand::TypedField {
                type_id: schema_id as u16,
                field_idx,
                field_type_tag: field_type_to_tag(&field.field_type),
            },
            borrow_key,
            field_type_info: field.field_type.clone(),
        })
    }

    pub(super) fn compile_reference_expr(
        &mut self,
        expr: &Expr,
        span: shape_ast::ast::Span,
        mode: BorrowMode,
    ) -> Result<u32> {
        let borrow_id = match expr {
            Expr::Identifier(name, id_span) => self.compile_reference_identifier(name, *id_span, mode),
            Expr::PropertyAccess {
                object,
                property,
                optional: false,
                ..
            } => self.compile_reference_property_access(object, property, span, mode),
            Expr::IndexAccess {
                object,
                index,
                end_index: None,
                ..
            } => self.compile_reference_index_access(object, index, span, mode),
            _ => Err(ShapeError::SemanticError {
                message:
                    "`&` can only be applied to a place expression (variable, field access, or index access)".to_string(),
                location: Some(self.span_to_source_location(span)),
            }),
        }?;
        self.last_expr_schema = None;
        self.last_expr_type_info = None;
        self.last_expr_numeric_type = None;
        Ok(borrow_id)
    }

    pub(super) fn compile_reference_property_access(
        &mut self,
        object: &Expr,
        property: &str,
        span: shape_ast::ast::Span,
        mode: BorrowMode,
    ) -> Result<u32> {
        let Some(place) = self.try_resolve_typed_field_place(object, property) else {
            return Err(ShapeError::SemanticError {
                message:
                    "`&` can only be applied to a simple variable name or compile-time-resolved field access (e.g., `&x`, `&obj.a`, `&obj.nested.field`)".to_string(),
                location: Some(self.span_to_source_location(span)),
            });
        };

        if mode == BorrowMode::Exclusive {
            let is_const = if place.is_local {
                self.is_local_const(place.slot)
            } else {
                self.is_module_binding_const(place.slot)
            };
            if is_const {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot pass const variable '{}.{}' by exclusive reference",
                        place.root_name, property
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
        }

        // MIR analysis is the sole authority for borrow checking.
        let borrow_id = u32::MAX;

        let root_operand = if place.is_local {
            Operand::Local(place.slot)
        } else {
            Operand::ModuleBinding(place.slot)
        };
        self.emit(Instruction::new(OpCode::MakeRef, Some(root_operand)));

        // For chained access (a.b.c), emit MakeFieldRef for each nesting level.
        let field_chain = self.collect_property_access_chain(object, property);
        for field_operand in field_chain {
            self.emit(Instruction::new(OpCode::MakeFieldRef, Some(field_operand)));
        }

        Ok(borrow_id)
    }

    /// Collect the chain of typed field operands for a property access path.
    /// For `a.b.c`, returns [operand_for_b, operand_for_c].
    /// For `a.b` (flat), returns [operand_for_b].
    fn collect_property_access_chain(&self, object: &Expr, property: &str) -> Vec<Operand> {
        let mut chain = Vec::new();
        self.collect_property_chain_inner(object, &mut chain);
        // Add the leaf field operand
        if let Some(place) = self.try_resolve_typed_field_place(object, property) {
            chain.push(place.typed_operand);
        }
        chain
    }

    fn collect_property_chain_inner(&self, expr: &Expr, chain: &mut Vec<Operand>) {
        if let Expr::PropertyAccess {
            object: inner_object,
            property: inner_property,
            optional: false,
            ..
        } = expr
        {
            // Recurse into the inner object first
            self.collect_property_chain_inner(inner_object, chain);
            // Resolve this intermediate level
            if let Some(place) = self.try_resolve_typed_field_place(inner_object, inner_property) {
                chain.push(place.typed_operand);
            }
        }
        // Base case: Identifier — no extra field ref needed (handled by MakeRef)
    }

    pub(super) fn compile_reference_index_access(
        &mut self,
        object: &Expr,
        index: &Expr,
        span: shape_ast::ast::Span,
        mode: BorrowMode,
    ) -> Result<u32> {
        // Resolve the base object to a local or module binding for MakeRef.
        let (root_operand, is_const) = match object {
            Expr::Identifier(name, _id_span) => {
                if let Some(local_idx) = self.resolve_local(name) {
                    (Operand::Local(local_idx), self.is_local_const(local_idx))
                } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
                    if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                        (
                            Operand::ModuleBinding(binding_idx),
                            self.is_module_binding_const(binding_idx),
                        )
                    } else {
                        return Err(ShapeError::SemanticError {
                            message: "`&expr[i]` requires the base to be a resolvable variable"
                                .to_string(),
                            location: Some(self.span_to_source_location(span)),
                        });
                    }
                } else {
                    return Err(ShapeError::SemanticError {
                        message: format!("Cannot resolve variable '{}' for index reference", name),
                        location: Some(self.span_to_source_location(span)),
                    });
                }
            }
            _ => {
                // For arbitrary base expressions, compile into a temp local.
                self.compile_expr(object)?;
                let temp = self.declare_temp_local("__idx_ref_base_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(temp)),
                ));
                (Operand::Local(temp), false)
            }
        };

        if mode == BorrowMode::Exclusive && is_const {
            return Err(ShapeError::SemanticError {
                message: "Cannot create an exclusive index reference into a const variable"
                    .to_string(),
                location: Some(self.span_to_source_location(span)),
            });
        }

        // MIR analysis is the sole authority for borrow checking.
        let borrow_id = u32::MAX;

        // Emit MakeRef for the base array variable.
        self.emit(Instruction::new(OpCode::MakeRef, Some(root_operand)));
        // Compile the index expression (pushes index value onto stack).
        self.compile_expr(index)?;
        // MakeIndexRef pops [base_ref, index] and pushes a projected index reference.
        self.emit(Instruction::new(OpCode::MakeIndexRef, None));
        Ok(borrow_id)
    }

    pub(super) fn mark_reference_binding(&mut self, slot: u16, is_local: bool, is_exclusive: bool) {
        if is_local {
            self.reference_value_locals.insert(slot);
            if is_exclusive {
                self.exclusive_reference_value_locals.insert(slot);
            } else {
                self.exclusive_reference_value_locals.remove(&slot);
            }
        } else {
            self.reference_value_module_bindings.insert(slot);
            if is_exclusive {
                self.exclusive_reference_value_module_bindings.insert(slot);
            } else {
                self.exclusive_reference_value_module_bindings.remove(&slot);
            }
        }
        self.set_binding_storage_class(slot, is_local, BindingStorageClass::Reference);
    }

    pub(super) fn compile_expr_for_reference_binding(
        &mut self,
        expr: &shape_ast::ast::Expr,
    ) -> Result<Option<(u32, bool)>> {
        if self.expr_should_preserve_reference_binding(expr) {
            self.compile_expr_preserving_refs(expr)?;
            Ok(self
                .last_expr_reference_mode()
                .map(|mode| (u32::MAX, mode == BorrowMode::Exclusive)))
        } else {
            self.compile_expr(expr)?;
            Ok(None)
        }
    }

    fn binding_target_requires_reference_value(&self) -> bool {
        let Some(name) = self.pending_variable_name.as_deref() else {
            return false;
        };

        if let Some(local_idx) = self.resolve_local(name) {
            if self.reference_value_locals.contains(&local_idx) {
                return true;
            }
        } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name)
            && let Some(binding_idx) = self.module_bindings.get(&scoped_name)
            && self.reference_value_module_bindings.contains(binding_idx)
        {
            return true;
        }

        self.future_reference_use_name_scopes
            .iter()
            .rev()
            .any(|scope| scope.contains(name))
    }

    fn expr_should_preserve_reference_binding(&self, expr: &shape_ast::ast::Expr) -> bool {
        match expr {
            shape_ast::ast::Expr::Reference { .. } => true,
            shape_ast::ast::Expr::FunctionCall { .. } | shape_ast::ast::Expr::MethodCall { .. } => {
                self.binding_target_requires_reference_value()
            }
            shape_ast::ast::Expr::Identifier(name, _)
            | shape_ast::ast::Expr::PatternRef(name, _) => {
                if let Some(local_idx) = self.resolve_local(name) {
                    self.reference_value_locals.contains(&local_idx)
                        || (self.binding_target_requires_reference_value()
                            && self.ref_locals.contains(&local_idx))
                } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
                    self.module_bindings
                        .get(&scoped_name)
                        .is_some_and(|binding_idx| {
                            self.reference_value_module_bindings.contains(binding_idx)
                        })
                } else {
                    false
                }
            }
            shape_ast::ast::Expr::Block(block, _) => block
                .items
                .last()
                .is_some_and(|item| self.block_item_should_preserve_reference_binding(item)),
            shape_ast::ast::Expr::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                self.expr_should_preserve_reference_binding(then_expr)
                    || else_expr
                        .as_deref()
                        .is_some_and(|expr| self.expr_should_preserve_reference_binding(expr))
            }
            shape_ast::ast::Expr::If(if_expr, _) => {
                self.expr_should_preserve_reference_binding(&if_expr.then_branch)
                    || if_expr
                        .else_branch
                        .as_deref()
                        .is_some_and(|expr| self.expr_should_preserve_reference_binding(expr))
            }
            shape_ast::ast::Expr::Match(match_expr, _) => match_expr
                .arms
                .iter()
                .any(|arm| self.expr_should_preserve_reference_binding(&arm.body)),
            shape_ast::ast::Expr::Let(let_expr, _) => {
                self.expr_should_preserve_reference_binding(&let_expr.body)
            }
            _ => false,
        }
    }

    fn block_item_should_preserve_reference_binding(&self, item: &BlockItem) -> bool {
        match item {
            BlockItem::Expression(expr) => self.expr_should_preserve_reference_binding(expr),
            _ => false,
        }
    }

    pub(super) fn local_binding_is_reference_value(&self, slot: u16) -> bool {
        self.ref_locals.contains(&slot) || self.reference_value_locals.contains(&slot)
    }

    pub(super) fn local_reference_binding_is_exclusive(&self, slot: u16) -> bool {
        self.exclusive_ref_locals.contains(&slot)
            || self.exclusive_reference_value_locals.contains(&slot)
    }

    fn track_reference_binding_slot(&mut self, _slot: u16, _is_local: bool) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn bind_reference_value_slot(
        &mut self,
        slot: u16,
        is_local: bool,
        _name: &str,
        is_exclusive: bool,
        _borrow_id: u32,
    ) {
        self.mark_reference_binding(slot, is_local, is_exclusive);
        self.track_reference_binding_slot(slot, is_local);
        if is_local {
            self.type_tracker
                .set_local_type(slot, VariableTypeInfo::unknown());
        } else {
            self.type_tracker
                .set_binding_type(slot, VariableTypeInfo::unknown());
        }
    }

    pub(super) fn bind_untracked_reference_value_slot(
        &mut self,
        slot: u16,
        is_local: bool,
        is_exclusive: bool,
    ) {
        self.mark_reference_binding(slot, is_local, is_exclusive);
        self.track_reference_binding_slot(slot, is_local);
        if is_local {
            self.type_tracker
                .set_local_type(slot, VariableTypeInfo::unknown());
        } else {
            self.type_tracker
                .set_binding_type(slot, VariableTypeInfo::unknown());
        }
    }

    pub(super) fn release_tracked_reference_borrow(&mut self, _slot: u16, _is_local: bool) {
        // Lexical borrow tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn clear_reference_binding(&mut self, slot: u16, is_local: bool) {
        self.release_tracked_reference_borrow(slot, is_local);
        if is_local {
            self.reference_value_locals.remove(&slot);
            self.exclusive_reference_value_locals.remove(&slot);
        } else {
            self.reference_value_module_bindings.remove(&slot);
            self.exclusive_reference_value_module_bindings.remove(&slot);
        }
        let fallback_storage = self.default_binding_storage_class_for_slot(slot, is_local);
        self.set_binding_storage_class(slot, is_local, fallback_storage);
    }

    pub(super) fn update_reference_binding_from_expr(
        &mut self,
        slot: u16,
        is_local: bool,
        expr: &shape_ast::ast::Expr,
    ) {
        if let shape_ast::ast::Expr::Reference { is_mutable, .. } = expr {
            self.clear_reference_binding(slot, is_local);
            self.bind_untracked_reference_value_slot(slot, is_local, *is_mutable);
        } else {
            self.clear_reference_binding(slot, is_local);
        }
    }

    pub(super) fn finish_reference_binding_from_expr(
        &mut self,
        slot: u16,
        is_local: bool,
        name: &str,
        expr: &shape_ast::ast::Expr,
        ref_borrow: Option<(u32, bool)>,
    ) {
        if let Some((borrow_id, is_exclusive)) = ref_borrow {
            self.clear_reference_binding(slot, is_local);
            if borrow_id == u32::MAX {
                self.bind_untracked_reference_value_slot(slot, is_local, is_exclusive);
            } else {
                self.bind_reference_value_slot(slot, is_local, name, is_exclusive, borrow_id);
            }
        } else {
            self.update_reference_binding_from_expr(slot, is_local, expr);
        }
    }

    pub(super) fn callable_pass_modes_from_expr(
        &self,
        expr: &shape_ast::ast::Expr,
    ) -> Option<Vec<ParamPassMode>> {
        match expr {
            shape_ast::ast::Expr::FunctionExpr { params, body, .. } => {
                Some(self.effective_function_like_pass_modes(None, params, Some(body)))
            }
            shape_ast::ast::Expr::Identifier(name, _)
            | shape_ast::ast::Expr::PatternRef(name, _) => self.callable_pass_modes_for_name(name),
            _ => None,
        }
    }

    fn callable_return_reference_summary_from_function_expr(
        &self,
        params: &[shape_ast::ast::FunctionParameter],
        body: &[Statement],
        span: shape_ast::ast::Span,
    ) -> Option<FunctionReturnReferenceSummary> {
        let mut effective_params = params.to_vec();
        let pass_modes = self.effective_function_like_pass_modes(None, params, Some(body));
        for (param, pass_mode) in effective_params.iter_mut().zip(pass_modes) {
            param.is_reference = pass_mode.is_reference();
            param.is_mut_reference = pass_mode.is_exclusive();
        }

        let lowering = crate::mir::lowering::lower_function_detailed(
            "__callable_expr__",
            &effective_params,
            body,
            span,
        );
        if lowering.had_fallbacks {
            return None;
        }

        let callee_summaries = self.build_callee_summaries(None, &lowering.all_local_names);
        crate::mir::solver::analyze(&lowering.mir, &callee_summaries)
            .return_reference_summary
            .map(Into::into)
    }

    pub(super) fn callable_return_reference_summary_from_expr(
        &self,
        expr: &shape_ast::ast::Expr,
    ) -> Option<FunctionReturnReferenceSummary> {
        match expr {
            shape_ast::ast::Expr::FunctionExpr {
                params, body, span, ..
            } => self.callable_return_reference_summary_from_function_expr(params, body, *span),
            shape_ast::ast::Expr::Identifier(name, _)
            | shape_ast::ast::Expr::PatternRef(name, _) => {
                self.function_return_reference_summary_for_name(name)
            }
            _ => None,
        }
    }

    pub(super) fn callable_pass_modes_for_name(&self, name: &str) -> Option<Vec<ParamPassMode>> {
        if let Some(local_idx) = self.resolve_local(name) {
            self.local_callable_pass_modes.get(&local_idx).cloned()
        } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
            let binding_idx = *self.module_bindings.get(&scoped_name)?;
            self.module_binding_callable_pass_modes
                .get(&binding_idx)
                .cloned()
        } else if let Some(func_idx) = self.find_function(name) {
            let func = &self.program.functions[func_idx];
            Some(Self::pass_modes_from_ref_flags(
                &func.ref_params,
                &func.ref_mutates,
            ))
        } else {
            None
        }
    }

    pub(super) fn function_return_reference_summary_for_name(
        &self,
        name: &str,
    ) -> Option<FunctionReturnReferenceSummary> {
        if let Some(local_idx) = self.resolve_local(name) {
            self.local_callable_return_reference_summaries
                .get(&local_idx)
                .cloned()
        } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
            let binding_idx = *self.module_bindings.get(&scoped_name)?;
            self.module_binding_callable_return_reference_summaries
                .get(&binding_idx)
                .cloned()
        } else {
            self.function_return_reference_summaries.get(name).cloned()
        }
    }

    pub(super) fn update_callable_binding_from_expr(
        &mut self,
        slot: u16,
        is_local: bool,
        expr: &shape_ast::ast::Expr,
    ) {
        let pass_modes = self.callable_pass_modes_from_expr(expr);
        let return_summary = self.callable_return_reference_summary_from_expr(expr);
        if is_local {
            if let Some(pass_modes) = pass_modes {
                self.local_callable_pass_modes.insert(slot, pass_modes);
            } else {
                self.local_callable_pass_modes.remove(&slot);
            }
            if let Some(return_summary) = return_summary {
                self.local_callable_return_reference_summaries
                    .insert(slot, return_summary);
            } else {
                self.local_callable_return_reference_summaries.remove(&slot);
            }
        } else if let Some(pass_modes) = pass_modes {
            self.module_binding_callable_pass_modes
                .insert(slot, pass_modes);
            if let Some(return_summary) = return_summary {
                self.module_binding_callable_return_reference_summaries
                    .insert(slot, return_summary);
            } else {
                self.module_binding_callable_return_reference_summaries
                    .remove(&slot);
            }
        } else {
            self.module_binding_callable_pass_modes.remove(&slot);
            self.module_binding_callable_return_reference_summaries
                .remove(&slot);
        }
    }

    pub(super) fn clear_callable_binding(&mut self, slot: u16, is_local: bool) {
        if is_local {
            self.local_callable_pass_modes.remove(&slot);
            self.local_callable_return_reference_summaries.remove(&slot);
        } else {
            self.module_binding_callable_pass_modes.remove(&slot);
            self.module_binding_callable_return_reference_summaries
                .remove(&slot);
        }
    }

    pub(super) fn push_module_reference_scope(&mut self) {
        // Lexical reference scope tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn pop_module_reference_scope(&mut self) {
        // Lexical reference scope tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn collect_reference_use_names_from_expr(
        &self,
        expr: &Expr,
        preserve_result: bool,
        names: &mut HashSet<String>,
    ) {
        match expr {
            Expr::Identifier(name, _) | Expr::PatternRef(name, _) => {
                if preserve_result {
                    names.insert(name.clone());
                }
            }
            Expr::Assign(assign, _) => {
                if let Expr::Identifier(name, _) = assign.target.as_ref() {
                    names.insert(name.clone());
                } else {
                    self.collect_reference_use_names_from_expr(
                        assign.target.as_ref(),
                        false,
                        names,
                    );
                }
                self.collect_reference_use_names_from_expr(&assign.value, false, names);
            }
            Expr::FunctionCall {
                name: callee,
                args,
                named_args,
                ..
            } => {
                let pass_modes = self.callable_pass_modes_for_name(callee);
                for (idx, arg) in args.iter().enumerate() {
                    let preserve_arg = pass_modes
                        .as_ref()
                        .and_then(|modes| modes.get(idx))
                        .is_some_and(|mode| mode.is_reference());
                    self.collect_reference_use_names_from_expr(arg, preserve_arg, names);
                }
                for (_, arg) in named_args {
                    self.collect_reference_use_names_from_expr(arg, false, names);
                }
            }
            Expr::MethodCall {
                receiver,
                args,
                named_args,
                ..
            } => {
                self.collect_reference_use_names_from_expr(receiver, false, names);
                for arg in args {
                    self.collect_reference_use_names_from_expr(arg, false, names);
                }
                for (_, arg) in named_args {
                    self.collect_reference_use_names_from_expr(arg, false, names);
                }
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.collect_reference_use_names_from_expr(condition, false, names);
                self.collect_reference_use_names_from_expr(then_expr, preserve_result, names);
                if let Some(else_expr) = else_expr.as_deref() {
                    self.collect_reference_use_names_from_expr(else_expr, preserve_result, names);
                }
            }
            Expr::If(if_expr, _) => {
                self.collect_reference_use_names_from_expr(&if_expr.condition, false, names);
                self.collect_reference_use_names_from_expr(
                    &if_expr.then_branch,
                    preserve_result,
                    names,
                );
                if let Some(else_branch) = if_expr.else_branch.as_deref() {
                    self.collect_reference_use_names_from_expr(else_branch, preserve_result, names);
                }
            }
            Expr::Match(match_expr, _) => {
                self.collect_reference_use_names_from_expr(&match_expr.scrutinee, false, names);
                for arm in &match_expr.arms {
                    if let Some(guard) = arm.guard.as_ref() {
                        self.collect_reference_use_names_from_expr(guard, false, names);
                    }
                    self.collect_reference_use_names_from_expr(&arm.body, preserve_result, names);
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    self.collect_reference_use_names_from_block_item(item, names);
                }
                if preserve_result && let Some(BlockItem::Expression(expr)) = block.items.last() {
                    self.collect_reference_use_names_from_expr(expr, true, names);
                }
            }
            Expr::Let(let_expr, _) => {
                if let Some(value) = &let_expr.value {
                    self.collect_reference_use_names_from_expr(value, false, names);
                }
                self.collect_reference_use_names_from_expr(&let_expr.body, preserve_result, names);
            }
            Expr::Array(items, _) => {
                for item in items {
                    self.collect_reference_use_names_from_expr(item, false, names);
                }
            }
            Expr::TableRows(rows, _) => {
                for row in rows {
                    for value in row {
                        self.collect_reference_use_names_from_expr(value, false, names);
                    }
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        shape_ast::ast::ObjectEntry::Field { value, .. } => {
                            self.collect_reference_use_names_from_expr(value, false, names);
                        }
                        shape_ast::ast::ObjectEntry::Spread(expr) => {
                            self.collect_reference_use_names_from_expr(expr, false, names);
                        }
                    }
                }
            }
            Expr::UnaryOp { operand, .. }
            | Expr::Spread(operand, _)
            | Expr::TryOperator(operand, _)
            | Expr::Await(operand, _)
            | Expr::TimeframeContext { expr: operand, .. }
            | Expr::UsingImpl { expr: operand, .. }
            | Expr::Reference { expr: operand, .. }
            | Expr::InstanceOf { expr: operand, .. } => {
                self.collect_reference_use_names_from_expr(operand, false, names);
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                self.collect_reference_use_names_from_expr(left, false, names);
                self.collect_reference_use_names_from_expr(right, false, names);
            }
            Expr::PropertyAccess { object, .. } => {
                self.collect_reference_use_names_from_expr(object, false, names);
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                self.collect_reference_use_names_from_expr(object, false, names);
                self.collect_reference_use_names_from_expr(index, false, names);
                if let Some(end_index) = end_index.as_deref() {
                    self.collect_reference_use_names_from_expr(end_index, false, names);
                }
            }
            _ => {}
        }
    }

    fn collect_reference_use_names_from_block_item(
        &self,
        item: &BlockItem,
        names: &mut HashSet<String>,
    ) {
        match item {
            BlockItem::VariableDecl(decl) => {
                if let Some(value) = &decl.value {
                    self.collect_reference_use_names_from_expr(value, false, names);
                }
            }
            BlockItem::Assignment(assign) => {
                if let Some(name) = assign.pattern.as_identifier() {
                    names.insert(name.to_string());
                }
                self.collect_reference_use_names_from_expr(&assign.value, false, names);
            }
            BlockItem::Statement(stmt) => {
                self.collect_reference_use_names_from_statement(stmt, names)
            }
            BlockItem::Expression(expr) => {
                self.collect_reference_use_names_from_expr(expr, false, names);
            }
        }
    }

    fn collect_reference_use_names_from_statement(
        &self,
        stmt: &Statement,
        names: &mut HashSet<String>,
    ) {
        use shape_ast::ast::ForInit;

        match stmt {
            Statement::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    self.collect_reference_use_names_from_expr(value, false, names);
                }
            }
            Statement::Assignment(assign, _) => {
                if let Some(name) = assign.pattern.as_identifier() {
                    names.insert(name.to_string());
                }
                self.collect_reference_use_names_from_expr(&assign.value, false, names);
            }
            Statement::Expression(expr, _) => {
                self.collect_reference_use_names_from_expr(expr, false, names);
            }
            Statement::Return(Some(expr), _) => {
                self.collect_reference_use_names_from_expr(expr, true, names);
            }
            Statement::If(if_stmt, _) => {
                self.collect_reference_use_names_from_expr(&if_stmt.condition, false, names);
                for stmt in &if_stmt.then_body {
                    self.collect_reference_use_names_from_statement(stmt, names);
                }
                if let Some(else_body) = if_stmt.else_body.as_ref() {
                    for stmt in else_body {
                        self.collect_reference_use_names_from_statement(stmt, names);
                    }
                }
            }
            Statement::While(while_loop, _) => {
                self.collect_reference_use_names_from_expr(&while_loop.condition, false, names);
                for stmt in &while_loop.body {
                    self.collect_reference_use_names_from_statement(stmt, names);
                }
            }
            Statement::For(for_loop, _) => {
                match &for_loop.init {
                    ForInit::ForIn { iter, .. } => {
                        self.collect_reference_use_names_from_expr(iter, false, names);
                    }
                    ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        self.collect_reference_use_names_from_statement(init, names);
                        self.collect_reference_use_names_from_expr(condition, false, names);
                        self.collect_reference_use_names_from_expr(update, false, names);
                    }
                }
                for stmt in &for_loop.body {
                    self.collect_reference_use_names_from_statement(stmt, names);
                }
            }
            Statement::Extend(ext, _) => {
                for method in &ext.methods {
                    for stmt in &method.body {
                        self.collect_reference_use_names_from_statement(stmt, names);
                    }
                }
            }
            Statement::SetParamValue { expression, .. }
            | Statement::SetReturnExpr { expression, .. }
            | Statement::ReplaceBodyExpr { expression, .. }
            | Statement::ReplaceModuleExpr { expression, .. } => {
                self.collect_reference_use_names_from_expr(expression, false, names);
            }
            Statement::ReplaceBody { body, .. } => {
                for stmt in body {
                    self.collect_reference_use_names_from_statement(stmt, names);
                }
            }
            Statement::Break(_)
            | Statement::Continue(_)
            | Statement::Return(None, _)
            | Statement::RemoveTarget(_)
            | Statement::SetParamType { .. }
            | Statement::SetReturnType { .. } => {}
        }
    }

    fn collect_reference_use_names_from_item(&self, item: &Item, names: &mut HashSet<String>) {
        match item {
            Item::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    self.collect_reference_use_names_from_expr(value, false, names);
                }
            }
            Item::Assignment(assign, _) => {
                if let Some(name) = assign.pattern.as_identifier() {
                    names.insert(name.to_string());
                }
                self.collect_reference_use_names_from_expr(&assign.value, false, names);
            }
            Item::Expression(expr, _) => {
                self.collect_reference_use_names_from_expr(expr, false, names);
            }
            Item::Statement(stmt, _) => {
                self.collect_reference_use_names_from_statement(stmt, names)
            }
            Item::Function(func, _) => {
                for stmt in &func.body {
                    self.collect_reference_use_names_from_statement(stmt, names);
                }
            }
            Item::Module(module, _) => {
                for item in &module.items {
                    self.collect_reference_use_names_from_item(item, names);
                }
            }
            Item::Export(export, _) => {
                if let Some(decl) = export.source_decl.as_ref()
                    && let Some(value) = decl.value.as_ref()
                {
                    self.collect_reference_use_names_from_expr(value, false, names);
                }
                if let shape_ast::ast::ExportItem::Function(func_def) = &export.item {
                    for stmt in &func_def.body {
                        self.collect_reference_use_names_from_statement(stmt, names);
                    }
                }
            }
            Item::Extend(ext, _) => {
                for method in &ext.methods {
                    for stmt in &method.body {
                        self.collect_reference_use_names_from_statement(stmt, names);
                    }
                }
            }
            Item::Impl(impl_block, _) => {
                for method in &impl_block.methods {
                    for stmt in &method.body {
                        self.collect_reference_use_names_from_statement(stmt, names);
                    }
                }
            }
            Item::Comptime(stmts, _) => {
                for stmt in stmts {
                    self.collect_reference_use_names_from_statement(stmt, names);
                }
            }
            _ => {}
        }
    }

    pub(super) fn push_future_reference_use_names(&mut self, names: HashSet<String>) {
        self.future_reference_use_name_scopes.push(names);
    }

    pub(super) fn pop_future_reference_use_names(&mut self) {
        self.future_reference_use_name_scopes.pop();
    }

    pub(super) fn future_reference_use_names_for_remaining_statements(
        &self,
        remaining: &[Statement],
    ) -> HashSet<String> {
        let mut names = HashSet::new();
        for stmt in remaining {
            self.collect_reference_use_names_from_statement(stmt, &mut names);
        }
        names
    }

    pub(super) fn future_reference_use_names_for_remaining_block_items(
        &self,
        remaining: &[BlockItem],
    ) -> HashSet<String> {
        let mut names = HashSet::new();
        for item in remaining {
            self.collect_reference_use_names_from_block_item(item, &mut names);
        }
        names
    }

    pub(super) fn future_reference_use_names_for_remaining_items(
        &self,
        remaining: &[Item],
    ) -> HashSet<String> {
        let mut names = HashSet::new();
        for item in remaining {
            self.collect_reference_use_names_from_item(item, &mut names);
        }
        names
    }

    pub(super) fn push_repeating_reference_release_barrier(&mut self) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn pop_repeating_reference_release_barrier(&mut self) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    fn local_reference_release_is_barrier_protected(&self, _slot: u16) -> bool {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
        false
    }

    fn module_reference_release_is_barrier_protected(&self, _slot: u16) -> bool {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
        false
    }

    pub(super) fn check_write_allowed_in_current_context(
        &self,
        _place: BorrowPlace,
        _source_location: Option<SourceLocation>,
    ) -> Result<()> {
        Ok(()) // MIR analysis is the sole authority
    }

    pub(super) fn check_named_binding_write_allowed(
        &self,
        name: &str,
        source_location: Option<SourceLocation>,
    ) -> Result<()> {
        if let Some(local_idx) = self.resolve_local(name) {
            // Immutability/const checks always run — even when MIR is authoritative.
            // MIR authority only bypasses the borrow checker (aliasing) checks below.
            if self.is_local_const(local_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!("Cannot reassign const variable '{}'", name),
                    location: source_location,
                });
            }
            if self.is_local_immutable(local_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot reassign immutable variable '{}'. Use `let mut` or `var` for mutable bindings",
                        name
                    ),
                    location: source_location,
                });
            }
            return Ok(()); // MIR analysis is the sole authority for borrow checks
        }

        let scoped_name = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
            // Immutability/const checks always run.
            if self.is_module_binding_const(binding_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!("Cannot reassign const variable '{}'", name),
                    location: source_location,
                });
            }
            if self.is_module_binding_immutable(binding_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot reassign immutable variable '{}'. Use `let mut` or `var` for mutable bindings",
                        name
                    ),
                    location: source_location,
                });
            }
            return Ok(()); // MIR analysis is the sole authority for borrow checks
        }

        Ok(())
    }

    pub(super) fn release_unused_local_reference_borrows_for_remaining_statements(
        &mut self,
        _remaining: &[Statement],
    ) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn release_unused_local_reference_borrows_for_remaining_block_items(
        &mut self,
        _remaining: &[BlockItem],
    ) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn release_unused_module_reference_borrows_for_remaining_statements(
        &mut self,
        _remaining: &[Statement],
    ) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn release_unused_module_reference_borrows_for_remaining_block_items(
        &mut self,
        _remaining: &[BlockItem],
    ) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn release_unused_module_reference_borrows_for_remaining_items(
        &mut self,
        _remaining: &[Item],
    ) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }
}
