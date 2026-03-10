//! Pattern binding - binding pattern matches to variables

use crate::bytecode::{Constant, Instruction, NumericWidth, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::VariableTypeInfo;
use shape_ast::ast::{Pattern, PatternConstructorFields, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};

use crate::compiler::BytecodeCompiler;

impl BytecodeCompiler {
    fn resolve_typed_field_operand_binding(
        &self,
        schema_id: u32,
        field_name: &str,
    ) -> Option<Operand> {
        if schema_id > u16::MAX as u32 {
            return None;
        }
        self.type_tracker
            .schema_registry()
            .get_by_id(schema_id)
            .and_then(|schema| {
                schema
                    .get_field(field_name)
                    .map(|field| Operand::TypedField {
                        type_id: schema_id as u16,
                        field_idx: field.index as u16,
                        field_type_tag: field_type_to_tag(&field.field_type),
                    })
            })
    }

    pub(in crate::compiler) fn compile_pattern_binding(&mut self, pattern: &Pattern) -> Result<()> {
        match pattern {
            Pattern::Identifier(name) => {
                let local_idx = self.declare_local(name)?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(local_idx)),
                ));
                Ok(())
            }
            Pattern::Typed {
                name,
                type_annotation,
            } => {
                let value_local = self.declare_temp_local("__typed_pattern_value_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));

                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(value_local)),
                ));
                let type_const = self
                    .program
                    .add_constant(Constant::TypeAnnotation(type_annotation.clone()));
                self.emit(Instruction::new(
                    OpCode::TypeCheck,
                    Some(Operand::Const(type_const)),
                ));
                let ok_jump = self.emit_jump(OpCode::JumpIfTrue, 0);

                let msg = self
                    .program
                    .add_constant(Constant::String("Pattern match failed".to_string()));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(msg)),
                ));
                self.emit(Instruction::simple(OpCode::Throw));

                self.patch_jump(ok_jump);
                let local_idx = self.declare_local(name)?;
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(value_local)),
                ));
                // Emit StoreLocalTyped for width types (i8, u8, i16, etc.)
                if let TypeAnnotation::Basic(type_name) = type_annotation {
                    if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
                        self.emit(Instruction::new(
                            OpCode::StoreLocalTyped,
                            Some(Operand::TypedLocal(
                                local_idx,
                                NumericWidth::from_int_width(w),
                            )),
                        ));
                        return Ok(());
                    }
                }
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(local_idx)),
                ));
                Ok(())
            }
            Pattern::Wildcard => {
                self.emit(Instruction::simple(OpCode::Pop));
                Ok(())
            }
            Pattern::Literal(lit) => {
                self.compile_literal(lit)?;
                self.emit(Instruction::simple(OpCode::Eq));
                let ok_jump = self.emit_jump(OpCode::JumpIfTrue, 0);

                let msg = self
                    .program
                    .add_constant(Constant::String("Pattern match failed".to_string()));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(msg)),
                ));
                self.emit(Instruction::simple(OpCode::Throw));

                self.patch_jump(ok_jump);
                Ok(())
            }
            Pattern::Array(patterns) => {
                self.emit(Instruction::simple(OpCode::Dup));
                self.emit(Instruction::simple(OpCode::Length));
                let min_len = self
                    .program
                    .add_constant(Constant::Number(patterns.len() as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(min_len)),
                ));
                self.emit(Instruction::simple(OpCode::Lt));
                let ok_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

                let msg = self.program.add_constant(Constant::String(format!(
                    "Array pattern requires at least {} elements",
                    patterns.len()
                )));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(msg)),
                ));
                self.emit(Instruction::simple(OpCode::Throw));
                self.patch_jump(ok_jump);

                for (index, pat) in patterns.iter().enumerate() {
                    self.emit(Instruction::simple(OpCode::Dup));
                    let idx_const = self.program.add_constant(Constant::Number(index as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(idx_const)),
                    ));
                    self.emit(Instruction::simple(OpCode::GetProp));
                    self.compile_pattern_binding(pat)?;
                }
                self.emit(Instruction::simple(OpCode::Pop));
                Ok(())
            }
            Pattern::Object(fields) => {
                let value_local = self.declare_temp_local("__pattern_obj_value_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                let schema_id = self.last_expr_schema.ok_or_else(|| ShapeError::SemanticError {
                    message: "Object pattern binding requires compile-time known schema. Runtime property lookup is disabled.".to_string(),
                    location: None,
                })?;
                for (key, pat) in fields {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let operand = self
                        .resolve_typed_field_operand_binding(schema_id, key)
                        .ok_or_else(|| ShapeError::SemanticError {
                            message: format!(
                                "Field '{}' is not declared in object schema for pattern binding.",
                                key
                            ),
                            location: None,
                        })?;
                    self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    self.compile_pattern_binding(pat)?;
                }
                Ok(())
            }
            Pattern::Constructor { .. } => {
                let value_local = self.declare_temp_local("__pattern_value_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                if let Some(schema_id) = self.last_expr_schema {
                    self.type_tracker.set_local_type(
                        value_local,
                        VariableTypeInfo::known(schema_id, format!("__typed_obj_{}", schema_id)),
                    );
                }

                let mut fail_jumps = Vec::new();
                self.compile_pattern_check_local(pattern, value_local, &mut fail_jumps, None)?;
                let ok_jump = self.emit_jump(OpCode::Jump, 0);

                for jump in fail_jumps {
                    self.patch_jump(jump);
                }

                let msg = self
                    .program
                    .add_constant(Constant::String("Pattern match failed".to_string()));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(msg)),
                ));
                self.emit(Instruction::simple(OpCode::Throw));

                self.patch_jump(ok_jump);
                self.compile_match_binding_local(pattern, value_local)
            }
        }
    }

    pub(in crate::compiler) fn compile_match_binding(&mut self, pattern: &Pattern) -> Result<()> {
        let value_local = self.declare_temp_local("__match_value_")?;
        if let Some(schema_id) = self.last_expr_schema {
            self.type_tracker.set_local_type(
                value_local,
                VariableTypeInfo::known(schema_id, format!("__typed_obj_{}", schema_id)),
            );
        }
        // Propagate numeric type info from the scrutinee expression so that
        // match binding variables inherit the correct storage hint (e.g., Int64).
        self.propagate_initializer_type_to_slot(value_local, true, false);
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(value_local)),
        ));
        self.compile_match_binding_local(pattern, value_local)?;
        self.mark_value_pattern_bindings_immutable(pattern);
        self.apply_binding_semantics_to_value_pattern_bindings(
            pattern,
            Self::owned_immutable_binding_semantics(),
        );
        Ok(())
    }

    pub(in crate::compiler) fn compile_match_binding_local(
        &mut self,
        pattern: &Pattern,
        value_local: u16,
    ) -> Result<()> {
        match pattern {
            Pattern::Identifier(name) => {
                let local_idx = self.declare_local(name)?;
                // Propagate type info from the scrutinee to the binding variable
                // so that downstream expressions (e.g., function calls) can use
                // typed opcodes when the scrutinee type is known.
                if let Some(source_info) = self.type_tracker.get_local_type(value_local).cloned() {
                    self.type_tracker.set_local_type(local_idx, source_info);
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(local_idx)),
                ));
                Ok(())
            }
            Pattern::Typed { name, .. } => {
                let local_idx = self.declare_local(name)?;
                if let Some(source_info) = self.type_tracker.get_local_type(value_local).cloned() {
                    self.type_tracker.set_local_type(local_idx, source_info);
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(local_idx)),
                ));
                Ok(())
            }
            Pattern::Wildcard | Pattern::Literal(_) => Ok(()),
            Pattern::Array(patterns) => {
                for (idx, pat) in patterns.iter().enumerate() {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let idx_const = self.program.add_constant(Constant::Number(idx as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(idx_const)),
                    ));
                    self.emit(Instruction::simple(OpCode::GetProp));
                    let elem_local = self.declare_temp_local("__match_elem_")?;
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(elem_local)),
                    ));
                    self.compile_match_binding_local(pat, elem_local)?;
                }
                Ok(())
            }
            Pattern::Object(fields) => {
                let schema_id = self
                    .type_tracker
                    .get_local_type(value_local)
                    .and_then(|info| info.schema_id)
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: "Object match binding requires compile-time known schema. Runtime property lookup is disabled.".to_string(),
                        location: None,
                    })?;
                for (key, pat) in fields {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let operand = self.resolve_typed_field_operand_binding(schema_id, key).ok_or_else(|| {
                        ShapeError::SemanticError {
                            message: format!(
                                "Field '{}' is not declared in object schema for match binding.",
                                key
                            ),
                            location: None,
                        }
                    })?;
                    self.emit(Instruction::new(
                        OpCode::GetFieldTyped,
                        Some(operand),
                    ));
                    let field_local = self.declare_temp_local("__match_field_")?;
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(field_local)),
                    ));
                    self.compile_match_binding_local(pat, field_local)?;
                }
                Ok(())
            }
            Pattern::Constructor {
                enum_name,
                variant,
                fields,
            } => match (enum_name.as_deref(), variant.as_str()) {
                (Some("Option"), "None") | (None, "None") => Ok(()),
                (Some("Option"), "Some") | (None, "Some") => {
                    if let PatternConstructorFields::Tuple(pats) = fields {
                        if pats.len() == 1 {
                            let inner_local = self.declare_temp_local("__some_inner_")?;
                            self.emit(Instruction::new(
                                OpCode::LoadLocal,
                                Some(Operand::Local(value_local)),
                            ));
                            self.emit(Instruction::simple(OpCode::UnwrapOption));
                            self.emit(Instruction::new(
                                OpCode::StoreLocal,
                                Some(Operand::Local(inner_local)),
                            ));
                            return self.compile_match_binding_local(&pats[0], inner_local);
                        }
                    }
                    Ok(())
                }
                (Some("Result"), "Ok") | (None, "Ok") | (Some("Result"), "Err") | (None, "Err") => {
                    if let PatternConstructorFields::Tuple(pats) = fields {
                        if pats.len() != 1 {
                            return Ok(());
                        }
                        let inner_local = self.declare_temp_local("__match_inner_")?;
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        self.emit(Instruction::simple(if variant == "Ok" {
                            OpCode::UnwrapOk
                        } else {
                            OpCode::UnwrapErr
                        }));
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(inner_local)),
                        ));
                        return self.compile_match_binding_local(&pats[0], inner_local);
                    }
                    Ok(())
                }
                (Some(enum_name), _) => {
                    // Look up enum schema - must be registered (no generic fallback)
                    if let Some(schema) = self.type_tracker.schema_registry().get(enum_name) {
                        if schema.get_enum_info().is_some() {
                            return self.compile_typed_enum_binding(value_local, schema.id, fields);
                        }
                    }
                    Err(ShapeError::SemanticError {
                        message: format!(
                            "Enum pattern '{}' requires a registered enum schema. Generic fallback is disabled.",
                            enum_name
                        ),
                        location: None,
                    })
                }
                (None, _) => {
                    Err(ShapeError::SemanticError {
                        message: "Bare enum variant patterns require type-resolved enum context. Generic fallback is disabled.".to_string(),
                        location: None,
                    })
                }
            },
        }
    }

    /// Compile enum binding for TypedObject (optimized path)
    fn compile_typed_enum_binding(
        &mut self,
        value_local: u16,
        schema_id: u32,
        fields: &PatternConstructorFields,
    ) -> Result<()> {
        match fields {
            PatternConstructorFields::Unit => Ok(()),
            PatternConstructorFields::Tuple(patterns) => {
                // Payload fields are at __payload_0, __payload_1, etc.
                for (idx, pat) in patterns.iter().enumerate() {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    // GetFieldTyped for __payload_{idx} (Any type)
                    self.emit(Instruction::new(
                        OpCode::GetFieldTyped,
                        Some(Operand::TypedField {
                            type_id: schema_id as u16,
                            field_idx: (idx + 1) as u16, // field 0 is __variant
                            field_type_tag: crate::executor::typed_object_ops::FIELD_TAG_ANY,
                        }),
                    ));
                    let elem_local = self.declare_temp_local("__typed_enum_elem_")?;
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(elem_local)),
                    ));
                    self.compile_match_binding_local(pat, elem_local)?;
                }
                Ok(())
            }
            PatternConstructorFields::Struct(patterns) => {
                // For struct payloads, we access fields by index
                for (idx, (_key, pat)) in patterns.iter().enumerate() {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    // GetFieldTyped for __payload_{idx} (Any type)
                    self.emit(Instruction::new(
                        OpCode::GetFieldTyped,
                        Some(Operand::TypedField {
                            type_id: schema_id as u16,
                            field_idx: (idx + 1) as u16,
                            field_type_tag: crate::executor::typed_object_ops::FIELD_TAG_ANY,
                        }),
                    ));
                    let field_local = self.declare_temp_local("__typed_enum_field_")?;
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(field_local)),
                    ));
                    self.compile_match_binding_local(pat, field_local)?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler::BytecodeCompiler;
    use crate::type_tracking::{BindingOwnershipClass, BindingStorageClass};
    use shape_ast::ast::Pattern;

    #[test]
    fn test_value_pattern_bindings_get_owned_semantics_recursively() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let left = compiler.declare_local("left").expect("declare left");
        let right = compiler.declare_local("right").expect("declare right");
        let pattern = Pattern::Object(vec![
            ("lhs".to_string(), Pattern::Identifier("left".to_string())),
            (
                "rhs".to_string(),
                Pattern::Array(vec![Pattern::Identifier("right".to_string())]),
            ),
        ]);

        compiler.apply_binding_semantics_to_value_pattern_bindings(
            &pattern,
            BytecodeCompiler::owned_mutable_binding_semantics(),
        );

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(left)
                .map(|semantics| semantics.ownership_class),
            Some(BindingOwnershipClass::OwnedMutable)
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(left)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::Direct)
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(right)
                .map(|semantics| semantics.ownership_class),
            Some(BindingOwnershipClass::OwnedMutable)
        );
    }
}
