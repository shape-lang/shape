//! Assignment and let expression compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use shape_ast::ast::{Expr, Spanned};
use shape_ast::error::{Result, ShapeError};

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a let expression
    pub(super) fn compile_expr_let(&mut self, let_expr: &shape_ast::ast::LetExpr) -> Result<()> {
        self.push_scope();

        let mut future_names = std::collections::HashSet::new();
        self.collect_reference_use_names_from_expr(
            &let_expr.body,
            self.current_expr_result_mode() == crate::compiler::ExprResultMode::PreserveRef,
            &mut future_names,
        );
        self.push_future_reference_use_names(future_names);

        let compile_result = (|| -> Result<()> {
            let mut ref_borrow = None;
            if let Some(value) = &let_expr.value {
                let saved_pending_variable_name = self.pending_variable_name.clone();
                self.pending_variable_name = let_expr
                    .pattern
                    .as_simple_name()
                    .map(|name| name.to_string());
                let compile_result = self.compile_expr_for_reference_binding(value);
                self.pending_variable_name = saved_pending_variable_name;
                ref_borrow = compile_result?;
            } else {
                self.emit(Instruction::simple(OpCode::PushNull));
            }

            self.compile_pattern_binding(&let_expr.pattern)?;
            self.mark_value_pattern_bindings_immutable(&let_expr.pattern);
            self.apply_binding_semantics_to_value_pattern_bindings(
                &let_expr.pattern,
                Self::owned_immutable_binding_semantics(),
            );
            if let Some(name) = let_expr.pattern.as_simple_name()
                && let Some(local_idx) = self.resolve_local(name)
            {
                if let Some(value) = &let_expr.value {
                    self.finish_reference_binding_from_expr(
                        local_idx, true, name, value, ref_borrow,
                    );
                    self.update_callable_binding_from_expr(local_idx, true, value);
                } else {
                    self.clear_reference_binding(local_idx, true);
                    self.clear_callable_binding(local_idx, true);
                }
            }
            if self.current_expr_result_mode() == crate::compiler::ExprResultMode::PreserveRef {
                self.compile_expr_preserving_refs(&let_expr.body)?;
            } else {
                self.compile_expr(&let_expr.body)?;
            }

            Ok(())
        })();

        self.pop_future_reference_use_names();
        self.pop_scope();
        compile_result
    }

    /// Compile an assignment expression
    pub(super) fn compile_expr_assign(
        &mut self,
        assign_expr: &shape_ast::ast::AssignExpr,
    ) -> Result<()> {
        // Check for const reassignment (covers compound assignments like +=)
        if let Expr::Identifier(name, _) = assign_expr.target.as_ref() {
            if let Some(local_idx) = self.resolve_local(name) {
                if !self.current_binding_uses_mir_write_authority(true)
                    && self.const_locals.contains(&local_idx)
                {
                    return Err(ShapeError::SemanticError {
                        message: format!("Cannot reassign const variable '{}'", name),
                        location: None,
                    });
                }
            } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
                if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                    if !self.current_binding_uses_mir_write_authority(false)
                        && self.const_module_bindings.contains(&binding_idx)
                    {
                        return Err(ShapeError::SemanticError {
                            message: format!("Cannot reassign const variable '{}'", name),
                            location: None,
                        });
                    }
                }
            }
        }

        match assign_expr.target.as_ref() {
            Expr::Identifier(name, id_span) => {
                // Optimization: x = x.push(val) → ArrayPushLocal (O(1) in-place mutation)
                if let Expr::MethodCall {
                    receiver,
                    method,
                    args,
                    ..
                } = assign_expr.value.as_ref()
                {
                    if method == "push" && args.len() == 1 {
                        if let Expr::Identifier(recv_name, _) = receiver.as_ref() {
                            if recv_name == name {
                                let source_loc = self.span_to_source_location(*id_span);
                                if let Some(local_idx) = self.resolve_local(name) {
                                    if !self.ref_locals.contains(&local_idx) {
                                        self.check_named_binding_write_allowed(
                                            name,
                                            Some(source_loc),
                                        )?;
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
                                        self.plan_flexible_binding_storage_from_expr(
                                            local_idx,
                                            true,
                                            assign_expr.value.as_ref(),
                                        );
                                        // Push expression result (the updated array)
                                        self.emit(Instruction::new(
                                            OpCode::LoadLocal,
                                            Some(Operand::Local(local_idx)),
                                        ));
                                        return Ok(());
                                    }
                                } else {
                                    self.check_named_binding_write_allowed(name, Some(source_loc))?;
                                    // ModuleBinding variable: same optimization with ModuleBinding operand
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
                                    self.plan_flexible_binding_storage_from_expr(
                                        binding_idx,
                                        false,
                                        assign_expr.value.as_ref(),
                                    );
                                    // Push expression result (the updated array)
                                    self.emit(Instruction::new(
                                        OpCode::LoadModuleBinding,
                                        Some(Operand::ModuleBinding(binding_idx)),
                                    ));
                                    return Ok(());
                                }
                            }
                        }
                    }
                }

                let saved_pending_variable_name = self.pending_variable_name.clone();
                self.pending_variable_name = Some(name.clone());
                let compile_result = self.compile_expr_for_reference_binding(&assign_expr.value);
                self.pending_variable_name = saved_pending_variable_name;
                let ref_borrow = compile_result?;
                self.emit(Instruction::simple(OpCode::Dup));
                // Mutable closure captures: emit StoreClosure
                if let Some(&upvalue_idx) = self.mutable_closure_captures.get(name.as_str()) {
                    self.emit(Instruction::new(
                        OpCode::StoreClosure,
                        Some(Operand::Local(upvalue_idx)),
                    ));
                    return Ok(());
                }
                if let Some(local_idx) = self.resolve_local(name) {
                    if self.local_binding_is_reference_value(local_idx) {
                        if !self.local_reference_binding_is_exclusive(local_idx) {
                            return Err(ShapeError::SemanticError {
                                message: format!(
                                    "cannot assign through shared reference variable '{}'",
                                    name
                                ),
                                location: Some(self.span_to_source_location(*id_span)),
                            });
                        }
                        // Reference parameter or reference-valued binding: write through the reference
                        self.emit(Instruction::new(
                            OpCode::DerefStore,
                            Some(Operand::Local(local_idx)),
                        ));
                    } else {
                        // Borrow check: reject writes to borrowed variables
                        let source_loc = self.span_to_source_location(*id_span);
                        self.check_named_binding_write_allowed(name, Some(source_loc))?;
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                        // Patch StoreLocal → StoreLocalTyped for width-typed locals
                        // so reassignment truncates to the declared width.
                        if let Some(type_name) = self
                            .type_tracker
                            .get_local_type(local_idx)
                            .and_then(|info| info.type_name.as_deref())
                        {
                            if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
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
                    if !self.local_binding_is_reference_value(local_idx) {
                        self.finish_reference_binding_from_expr(
                            local_idx,
                            true,
                            name,
                            &assign_expr.value,
                            ref_borrow,
                        );
                        self.update_callable_binding_from_expr(local_idx, true, &assign_expr.value);
                    }
                    self.plan_flexible_binding_storage_from_expr(
                        local_idx,
                        true,
                        &assign_expr.value,
                    );
                } else {
                    let source_loc = self.span_to_source_location(*id_span);
                    self.check_named_binding_write_allowed(name, Some(source_loc))?;
                    let binding_idx = self.get_or_create_module_binding(name);
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                    // Patch StoreModuleBinding → StoreModuleBindingTyped for width-typed bindings
                    if let Some(type_name) = self
                        .type_tracker
                        .get_binding_type(binding_idx)
                        .and_then(|info| info.type_name.as_deref())
                    {
                        if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
                            if let Some(last) = self.program.instructions.last_mut() {
                                if last.opcode == OpCode::StoreModuleBinding {
                                    last.opcode = OpCode::StoreModuleBindingTyped;
                                    last.operand = Some(Operand::TypedModuleBinding(
                                        binding_idx,
                                        crate::bytecode::NumericWidth::from_int_width(w),
                                    ));
                                }
                            }
                        }
                    }
                    self.finish_reference_binding_from_expr(
                        binding_idx,
                        false,
                        name,
                        &assign_expr.value,
                        ref_borrow,
                    );
                    self.update_callable_binding_from_expr(binding_idx, false, &assign_expr.value);
                    self.plan_flexible_binding_storage_from_expr(
                        binding_idx,
                        false,
                        &assign_expr.value,
                    );
                }
                self.propagate_assignment_type_to_identifier(name);
                Ok(())
            }
            Expr::PropertyAccess {
                object, property, ..
            } => {
                const OBJECT_REF_STORAGE_ERROR: &str = "cannot store a reference in an object or struct literal — references are scoped borrows that cannot escape into aggregate values. Use owned values instead";
                if let Some(place) = self.try_resolve_typed_field_place(object, property) {
                    let label = format!("{}.{}", place.root_name, property);
                    let source_loc = self.span_to_source_location(assign_expr.target.span());
                    self.check_write_allowed_in_current_context(place.borrow_key, Some(source_loc))
                        .map_err(|err| Self::relabel_borrow_error(err, place.borrow_key, &label))?;

                    let field_ref = self.declare_temp_local("__field_assign_ref_")?;
                    let root_operand = if place.is_local {
                        Operand::Local(place.slot)
                    } else {
                        Operand::ModuleBinding(place.slot)
                    };
                    self.emit(Instruction::new(OpCode::MakeRef, Some(root_operand)));
                    self.emit(Instruction::new(
                        OpCode::MakeFieldRef,
                        Some(place.typed_operand),
                    ));
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(field_ref)),
                    ));

                    self.reject_direct_reference_storage(
                        &assign_expr.value,
                        OBJECT_REF_STORAGE_ERROR,
                    )?;
                    self.compile_expr(&assign_expr.value)?;
                    let value_local = self.declare_temp_local("__assign_value_")?;
                    self.emit(Instruction::simple(OpCode::Dup));
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::DerefStore,
                        Some(Operand::Local(field_ref)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    return Ok(());
                }

                if let Expr::Identifier(name, id_span) = object.as_ref()
                    && let Some(local_idx) = self.resolve_local(name)
                    && !self.ref_locals.contains(&local_idx)
                {
                    let source_loc = self.span_to_source_location(*id_span);
                    self.check_write_allowed_in_current_context(
                        Self::borrow_key_for_local(local_idx),
                        Some(source_loc),
                    )
                    .map_err(|e| match e {
                        ShapeError::SemanticError { message, location } => {
                            let user_msg = message
                                .replace(&format!("(slot {})", local_idx), &format!("'{}'", name));
                            ShapeError::SemanticError {
                                message: user_msg,
                                location,
                            }
                        }
                        other => other,
                    })?;
                }
                self.compile_expr(object)?;
                let Some(schema_id) = self.last_expr_schema else {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Assignment to '{}.{}' requires compile-time field resolution. Generic runtime property lookup is disabled.",
                            match object.as_ref() {
                                Expr::Identifier(name, _) => name,
                                _ => "<expr>",
                            },
                            property
                        ),
                        location: None,
                    });
                };

                let typed_operand = self
                    .type_tracker
                    .schema_registry()
                    .get_by_id(schema_id)
                    .and_then(|schema| {
                        schema.get_field(property).and_then(|field| {
                            if schema_id <= u16::MAX as u32 {
                                Some(Operand::TypedField {
                                    type_id: schema_id as u16,
                                    field_idx: field.index as u16,
                                    field_type_tag: field_type_to_tag(&field.field_type),
                                })
                            } else {
                                None
                            }
                        })
                    })
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: format!(
                            "Property '{}.{}' is not resolvable at compile time for assignment.",
                            match object.as_ref() {
                                Expr::Identifier(name, _) => name,
                                _ => "<expr>",
                            },
                            property
                        ),
                        location: None,
                    })?;

                self.reject_direct_reference_storage(&assign_expr.value, OBJECT_REF_STORAGE_ERROR)?;
                self.compile_expr(&assign_expr.value)?;
                let value_local = self.declare_temp_local("__assign_value_")?;
                self.emit(Instruction::simple(OpCode::Dup));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit(Instruction::new(OpCode::SetFieldTyped, Some(typed_operand)));
                // Store the modified object back through the property chain
                // (handles nested field mutation like o.data.val = 42)
                self.emit_nested_store_back(object)?;
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(value_local)),
                ));
                Ok(())
            }
            Expr::IndexAccess {
                object,
                index,
                end_index: None,
                ..
            } => {
                const ARRAY_REF_STORAGE_ERROR: &str = "cannot store a reference in an array — references are scoped borrows that cannot escape into collections. Use owned values instead";
                if let Expr::Identifier(name, _) = object.as_ref() {
                    self.compile_expr(index)?;
                    self.reject_direct_reference_storage(
                        &assign_expr.value,
                        ARRAY_REF_STORAGE_ERROR,
                    )?;
                    self.compile_expr(&assign_expr.value)?;
                    let value_local = self.declare_temp_local("__assign_value_")?;
                    self.emit(Instruction::simple(OpCode::Dup));
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    if let Some(local_idx) = self.resolve_local(name) {
                        if self.ref_locals.contains(&local_idx) {
                            // Reference parameter: mutate array in-place through the reference
                            self.emit(Instruction::new(
                                OpCode::SetIndexRef,
                                Some(Operand::Local(local_idx)),
                            ));
                        } else {
                            let source_loc = self.span_to_source_location(index.span());
                            self.check_write_allowed_in_current_context(
                                Self::borrow_key_for_local(local_idx),
                                Some(source_loc),
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
                            self.emit(Instruction::new(
                                OpCode::SetLocalIndex,
                                Some(Operand::Local(local_idx)),
                            ));
                        }
                    } else {
                        let binding_idx = self.get_or_create_module_binding(name);
                        let source_loc = self.span_to_source_location(index.span());
                        self.check_write_allowed_in_current_context(
                            Self::borrow_key_for_module_binding(binding_idx),
                            Some(source_loc),
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
                        self.emit(Instruction::new(
                            OpCode::SetModuleBindingIndex,
                            Some(Operand::ModuleBinding(binding_idx)),
                        ));
                    }
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    Ok(())
                } else {
                    self.compile_expr(object)?;
                    self.compile_expr(index)?;
                    self.reject_direct_reference_storage(
                        &assign_expr.value,
                        ARRAY_REF_STORAGE_ERROR,
                    )?;
                    self.compile_expr(&assign_expr.value)?;
                    let value_local = self.declare_temp_local("__assign_value_")?;
                    self.emit(Instruction::simple(OpCode::Dup));
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit(Instruction::simple(OpCode::SetProp));
                    self.emit(Instruction::simple(OpCode::Pop));
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    Ok(())
                }
            }
            Expr::IndexAccess {
                object,
                index,
                end_index: Some(end_index),
                ..
            } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.compile_expr(end_index)?;
                // Push inclusive flag (exclusive by default for slice syntax)
                let const_idx = self.program.add_constant(Constant::Bool(false));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(const_idx)),
                ));
                self.emit(Instruction::simple(OpCode::MakeRange));
                self.compile_expr(&assign_expr.value)?;
                let value_local = self.declare_temp_local("__assign_value_")?;
                self.emit(Instruction::simple(OpCode::Dup));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit(Instruction::simple(OpCode::SetProp));
                if let Expr::Identifier(name, _) = object.as_ref() {
                    self.emit_store_identifier(name)?;
                } else {
                    self.emit(Instruction::simple(OpCode::Pop));
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(value_local)),
                ));
                Ok(())
            }
            _ => Err(ShapeError::RuntimeError {
                message: "Invalid assignment target".to_string(),
                location: None,
            }),
        }
    }

    /// Store the modified object back through a property access chain.
    /// After SetFieldTyped, the modified object is on the stack. If the parent
    /// expression is an Identifier, store directly. If it's a nested
    /// PropertyAccess, recursively store back through each level.
    fn emit_nested_store_back(&mut self, object: &Expr) -> Result<()> {
        match object {
            Expr::Identifier(name, _) => {
                self.emit_store_identifier(name)?;
                Ok(())
            }
            Expr::PropertyAccess {
                object: parent,
                property,
                ..
            } => {
                // The modified child object is on the stack.
                // Store it to a temp so we can reload the parent.
                let child_temp = self.declare_temp_local("__nested_assign_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(child_temp)),
                ));

                // Load the parent object
                self.compile_expr(parent)?;
                let schema_id = self
                    .last_expr_schema
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: format!(
                            "Nested assignment requires compile-time schema for parent of '{}'.",
                            property
                        ),
                        location: None,
                    })?;

                // Load the modified child
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(child_temp)),
                ));

                // Set the field on the parent
                let typed_operand = self
                    .type_tracker
                    .schema_registry()
                    .get_by_id(schema_id)
                    .and_then(|schema| {
                        schema.get_field(property).and_then(|field| {
                            if schema_id <= u16::MAX as u32 {
                                Some(Operand::TypedField {
                                    type_id: schema_id as u16,
                                    field_idx: field.index as u16,
                                    field_type_tag: field_type_to_tag(&field.field_type),
                                })
                            } else {
                                None
                            }
                        })
                    })
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: format!(
                            "Property '{}' is not resolvable for nested store-back.",
                            property
                        ),
                        location: None,
                    })?;
                self.emit(Instruction::new(OpCode::SetFieldTyped, Some(typed_operand)));

                // Recurse up the chain
                self.emit_nested_store_back(parent)
            }
            _ => {
                self.emit(Instruction::simple(OpCode::Pop));
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler::BytecodeCompiler;
    use shape_ast::parser::parse_program;

    #[test]
    fn test_let_expression_binding_is_immutable() {
        let code = r#"
            function test() {
                return let x = 5 in {
                    x = 6
                    x
                }
            }
        "#;
        let program = parse_program(code).expect("parse failed");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "reassigning let-expression binding should fail"
        );
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("immutable variable 'x'"),
            "unexpected error: {}",
            err
        );
    }
}
