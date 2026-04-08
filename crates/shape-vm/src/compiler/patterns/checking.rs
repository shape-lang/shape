//! Pattern checking - verifying if values match patterns

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::VariableTypeInfo;
use shape_ast::ast::{Literal, Pattern, PatternConstructorFields};
use shape_ast::error::{Result, ShapeError};

use crate::compiler::BytecodeCompiler;
use super::helpers::typed_eq_opcode_for_literal;

// Reserved schema fields for TypedObject enum layout
// __variant at offset 0, __payload_N at offsets 8, 16, etc.

impl BytecodeCompiler {
    fn resolve_typed_field_operand_checking(
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

    pub(in crate::compiler) fn compile_pattern_check(
        &mut self,
        pattern: &Pattern,
        hint_span: Option<shape_ast::ast::Span>,
    ) -> Result<()> {
        self.push_scope();
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
        self.compile_pattern_check_local(pattern, value_local, &mut fail_jumps, hint_span)?;

        self.emit_bool(true);
        let end_jump = self.emit_jump(OpCode::Jump, 0);

        for jump in fail_jumps {
            self.patch_jump(jump);
        }
        self.emit_bool(false);
        self.patch_jump(end_jump);
        self.pop_scope();
        Ok(())
    }

    pub(in crate::compiler) fn compile_pattern_check_local(
        &mut self,
        pattern: &Pattern,
        value_local: u16,
        fail_jumps: &mut Vec<usize>,
        hint_span: Option<shape_ast::ast::Span>,
    ) -> Result<()> {
        match pattern {
            Pattern::Wildcard | Pattern::Identifier(_) => Ok(()),
            Pattern::Typed {
                type_annotation, ..
            } => {
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
                let jump = self.emit_jump(OpCode::JumpIfFalse, 0);
                fail_jumps.push(jump);
                Ok(())
            }
            Pattern::Literal(lit) => {
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(value_local)),
                ));

                // Bool patterns desugar to a direct conditional jump — no
                // equality opcode at all. The loaded scrutinee is itself
                // the bool we want to test.
                if let Literal::Bool(b) = lit {
                    let jump_op = if *b {
                        OpCode::JumpIfFalse
                    } else {
                        OpCode::JumpIfTrue
                    };
                    let jump = self.emit_jump(jump_op, 0);
                    fail_jumps.push(jump);
                    return Ok(());
                }

                self.compile_literal(lit)?;
                let eq_op = typed_eq_opcode_for_literal(lit).unwrap_or(OpCode::Eq);
                self.emit(Instruction::simple(eq_op));
                let jump = self.emit_jump(OpCode::JumpIfFalse, 0);
                fail_jumps.push(jump);
                Ok(())
            }
            Pattern::Array(patterns) => {
                self.emit_pattern_type_check(value_local, "array", fail_jumps)?;
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit(Instruction::simple(OpCode::Length));
                // Stage 2.6.4: Length pushes int (op_length emits ValueWord::from_i64),
                // so use Constant::Int + EqInt for the typed comparison.
                let expected_len = self
                    .program
                    .add_constant(Constant::Int(patterns.len() as i64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(expected_len)),
                ));
                self.emit(Instruction::simple(OpCode::EqInt));
                let jump = self.emit_jump(OpCode::JumpIfFalse, 0);
                fail_jumps.push(jump);

                for (idx, pat) in patterns.iter().enumerate() {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let idx_const = self.program.add_constant(Constant::Int(idx as i64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(idx_const)),
                    ));
                    self.emit(Instruction::simple(OpCode::GetProp));
                    let elem_local = self.declare_temp_local("__pattern_elem_")?;
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(elem_local)),
                    ));
                    self.compile_pattern_check_local(pat, elem_local, fail_jumps, hint_span)?;
                }
                Ok(())
            }
            Pattern::Object(fields) => {
                self.emit_pattern_type_check(value_local, "object", fail_jumps)?;
                let schema_id = self
                    .type_tracker
                    .get_local_type(value_local)
                    .and_then(|info| info.schema_id)
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: "Object pattern checking requires compile-time known schema. Runtime property lookup is disabled.".to_string(),
                        location: hint_span.map(|s| self.span_to_source_location(s)),
                    })?;
                for (key, pat) in fields {
                    let field_local = self.declare_temp_local("__pattern_field_")?;
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let operand = self
                        .resolve_typed_field_operand_checking(schema_id, key)
                        .ok_or_else(|| ShapeError::SemanticError {
                            message: format!(
                                "Field '{}' is not declared in object schema for pattern checking.",
                                key
                            ),
                            location: hint_span.map(|s| self.span_to_source_location(s)),
                        })?;
                    self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(field_local)),
                    ));
                    self.compile_pattern_check_local(pat, field_local, fail_jumps, hint_span)?;
                }
                Ok(())
            }
            Pattern::Constructor {
                enum_name,
                variant,
                fields,
            } => {
                match (enum_name.as_deref(), variant.as_str()) {
                    (Some("Option"), "None") | (None, "None") => {
                        if !matches!(fields, PatternConstructorFields::Unit) {
                            fail_jumps.push(self.emit_jump(OpCode::Jump, 0));
                            return Ok(());
                        }
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        self.emit(Instruction::simple(OpCode::PushNull));
                        self.emit(Instruction::simple(OpCode::Eq));
                        let jump = self.emit_jump(OpCode::JumpIfFalse, 0);
                        fail_jumps.push(jump);
                        Ok(())
                    }
                    (Some("Option"), "Some") | (None, "Some") => {
                        let field_pats = match fields {
                            PatternConstructorFields::Tuple(pats) if pats.len() == 1 => pats,
                            _ => {
                                fail_jumps.push(self.emit_jump(OpCode::Jump, 0));
                                return Ok(());
                            }
                        };
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        self.emit(Instruction::simple(OpCode::PushNull));
                        self.emit(Instruction::simple(OpCode::Eq));
                        let jump = self.emit_jump(OpCode::JumpIfTrue, 0);
                        fail_jumps.push(jump);
                        self.compile_pattern_check_local(
                            &field_pats[0],
                            value_local,
                            fail_jumps,
                            hint_span,
                        )
                    }
                    (Some("Result"), "Ok")
                    | (None, "Ok")
                    | (Some("Result"), "Err")
                    | (None, "Err") => {
                        let field_pats = match fields {
                            PatternConstructorFields::Tuple(pats) if pats.len() == 1 => pats,
                            _ => {
                                fail_jumps.push(self.emit_jump(OpCode::Jump, 0));
                                return Ok(());
                            }
                        };
                        let inner_local = self.declare_temp_local("__pattern_inner_")?;
                        let is_variant_opcode = if variant == "Ok" {
                            OpCode::IsOk
                        } else {
                            OpCode::IsErr
                        };
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        self.emit(Instruction::simple(is_variant_opcode));
                        let fail_jump = self.emit_jump(OpCode::JumpIfFalse, 0);
                        fail_jumps.push(fail_jump);

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
                        self.compile_pattern_check_local(
                            &field_pats[0],
                            inner_local,
                            fail_jumps,
                            hint_span,
                        )?;
                        Ok(())
                    }
                    (Some(enum_name), _) => {
                        // Look up enum schema - must be registered
                        let resolved_name = self.resolve_type_name(enum_name);
                        let schema = self.type_tracker.schema_registry().get(resolved_name.as_str());
                        let enum_info = schema.and_then(|s| s.get_enum_info());
                        let variant_info = enum_info.and_then(|e| e.variant_by_name(variant));

                        if let (Some(schema), Some(variant_info)) = (schema, variant_info) {
                            self.compile_typed_enum_pattern_check(
                                value_local,
                                schema.id,
                                variant_info.id,
                                fields,
                                fail_jumps,
                                hint_span,
                            )
                        } else if schema.is_none() {
                            Err(ShapeError::SemanticError {
                                message: format!(
                                    "Unknown enum type '{}'. Make sure it is imported or defined.",
                                    enum_name
                                ),
                                location: hint_span.map(|s| self.span_to_source_location(s)),
                            })
                        } else {
                            Err(ShapeError::SemanticError {
                                message: format!(
                                    "Unknown variant '{}' for enum '{}'",
                                    variant, enum_name
                                ),
                                location: hint_span.map(|s| self.span_to_source_location(s)),
                            })
                        }
                    }
                    _ => {
                        fail_jumps.push(self.emit_jump(OpCode::Jump, 0));
                        Ok(())
                    }
                }
            }
        }
    }

    /// Compile pattern check for TypedObject enum (optimized path)
    ///
    /// Uses GetFieldTyped for direct memory access to __variant field,
    /// then numeric comparison with expected variant_id.
    fn compile_typed_enum_pattern_check(
        &mut self,
        value_local: u16,
        schema_id: u32,
        expected_variant_id: u16,
        fields: &PatternConstructorFields,
        fail_jumps: &mut Vec<usize>,
        hint_span: Option<shape_ast::ast::Span>,
    ) -> Result<()> {
        // Load the value
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(value_local)),
        ));

        // Get __variant field using GetFieldTyped (I64 type)
        self.emit(Instruction::new(
            OpCode::GetFieldTyped,
            Some(Operand::TypedField {
                type_id: schema_id as u16,
                field_idx: 0,
                field_type_tag: crate::executor::typed_object_ops::FIELD_TAG_I64,
            }),
        ));

        // Compare to expected variant_id (stored as i64 in __variant).
        // Stage 2.6.4: __variant is always i64, so use EqInt directly.
        let expected_const = self
            .program
            .add_constant(Constant::Int(expected_variant_id as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(expected_const)),
        ));
        self.emit(Instruction::simple(OpCode::EqInt));
        let jump = self.emit_jump(OpCode::JumpIfFalse, 0);
        fail_jumps.push(jump);

        // Handle payload fields
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
                    self.compile_pattern_check_local(pat, elem_local, fail_jumps, hint_span)?;
                }
                Ok(())
            }
            PatternConstructorFields::Struct(patterns) => {
                // For struct payloads, we access fields by index
                // The order matches the struct definition order
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
                    self.compile_pattern_check_local(pat, field_local, fail_jumps, hint_span)?;
                }
                Ok(())
            }
        }
    }
}
