//! Pattern binding - binding pattern matches to variables

use crate::bytecode::{Constant, Instruction, NumericWidth, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::VariableTypeInfo;
use shape_ast::ast::{Literal, Pattern, PatternConstructorFields, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};

use super::helpers::typed_eq_opcode_for_literal;
use crate::compiler::BytecodeCompiler;

/// Tracker type-name for a `ConcreteType` payload binder (F5). Scalars map
/// to their Shape surface names; arrays/hashmaps to the `Vec<…>` / `HashMap<…>`
/// tracker-name forms the type tracker and `iter_element_type_name` recognise;
/// named struct/enum types to their name. Returns `None` for shapes with no
/// stable tracker name (tuple/function/etc.) — the caller then leaves the
/// element/map side-table stamp it already recorded in place.
pub(crate) fn concrete_type_tracker_name(ct: &shape_value::v2::ConcreteType) -> Option<String> {
    use shape_value::v2::ConcreteType;
    match ct {
        ConcreteType::I64 => Some("int".to_string()),
        ConcreteType::F64 => Some("number".to_string()),
        ConcreteType::Bool => Some("bool".to_string()),
        ConcreteType::String => Some("string".to_string()),
        ConcreteType::Decimal => Some("decimal".to_string()),
        ConcreteType::Array(elem) => {
            concrete_type_tracker_name(elem).map(|inner| format!("Vec<{inner}>"))
        }
        ConcreteType::Struct(layout) => layout.name.as_ref().map(|n| n.to_string()),
        ConcreteType::Enum(layout) => layout.name.as_ref().map(|n| n.to_string()),
        _ => None,
    }
}

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
                // Stage 2.6.4: Bool patterns desugar to direct conditional
                // jump (the scrutinee on top of stack IS the bool to test).
                if let Literal::Bool(b) = lit {
                    let jump_op = if *b {
                        OpCode::JumpIfTrue
                    } else {
                        OpCode::JumpIfFalse
                    };
                    let ok_jump = self.emit_jump(jump_op, 0);
                    let msg = self
                        .program
                        .add_constant(Constant::String("Pattern match failed".to_string()));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(msg)),
                    ));
                    self.emit(Instruction::simple(OpCode::Throw));
                    self.patch_jump(ok_jump);
                    return Ok(());
                }

                self.compile_literal(lit)?;
                let eq_op =
                    typed_eq_opcode_for_literal(lit).ok_or_else(|| ShapeError::SemanticError {
                        message: format!(
                            "Pattern matching on {} literals is not yet supported",
                            lit
                        ),
                        location: None,
                    })?;
                self.emit(Instruction::simple(eq_op));
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
                // Stage 2.6.4: Length pushes int, so use Constant::Int + LtInt.
                let min_len = self
                    .program
                    .add_constant(Constant::Int(patterns.len() as i64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(min_len)),
                ));
                self.emit(Instruction::simple(OpCode::LtInt));
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
                    let idx_const = self.program.add_constant(Constant::Int(index as i64));
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

    pub(in crate::compiler) fn compile_match_binding(
        &mut self,
        pattern: &Pattern,
        scrutinee_ct: Option<&shape_value::v2::ConcreteType>,
    ) -> Result<()> {
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
        // F5 (v0.3.3 strict-flip): record the scrutinee's proven ConcreteType
        // on the match-value temp so `Ok(v)` / `Some(v)` / `Err(e)` payload
        // unwraps can stamp the binder type (`stamp_unwrapped_payload_local`).
        if let Some(ct) = scrutinee_ct {
            self.current_function_local_concrete_types
                .insert(value_local, ct.clone());
        }
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
                    let operand = self
                        .resolve_typed_field_operand_binding(schema_id, key)
                        .ok_or_else(|| ShapeError::SemanticError {
                            message: format!(
                                "Field '{}' is not declared in object schema for match binding.",
                                key
                            ),
                            location: None,
                        })?;
                    self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    let field_local = self.declare_temp_local("__match_field_")?;
                    // Phase 3e: propagate the schema field's type onto the
                    // temp local so the downstream binding (Pattern::Identifier
                    // or Pattern::Typed) inherits the type info via the
                    // existing source_info copy in those arms. Without this,
                    // `match obj { { x, y } => x + y }` over a TypedObject
                    // with int fields gets x and y as Unknown, breaking
                    // strict-typing.
                    if let Some(field_def) = self
                        .type_tracker
                        .schema_registry()
                        .get_by_id(schema_id)
                        .and_then(|s| s.get_field(key))
                    {
                        let field_ty = field_def.field_type.clone();
                        let type_name_opt = match &field_ty {
                            shape_runtime::type_schema::FieldType::I64 => Some("int"),
                            shape_runtime::type_schema::FieldType::F64 => Some("number"),
                            shape_runtime::type_schema::FieldType::Bool => Some("bool"),
                            shape_runtime::type_schema::FieldType::String => Some("string"),
                            shape_runtime::type_schema::FieldType::Decimal => Some("decimal"),
                            _ => None,
                        };
                        if let Some(tn) = type_name_opt {
                            self.set_local_type_info(field_local, tn);
                        }
                    }
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
                            self.stamp_unwrapped_payload_local(value_local, inner_local, "Some");
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
                        self.stamp_unwrapped_payload_local(value_local, inner_local, variant);
                        return self.compile_match_binding_local(&pats[0], inner_local);
                    }
                    Ok(())
                }
                (Some(enum_name), _) => {
                    // Look up enum schema - must be registered (no generic fallback)
                    let resolved_name = self.resolve_type_name(enum_name);
                    if let Some(schema) = self
                        .type_tracker
                        .schema_registry()
                        .get(resolved_name.as_str())
                    {
                        if schema.get_enum_info().is_some() {
                            let schema_id = schema.id;
                            return self.compile_typed_enum_binding(
                                value_local,
                                schema_id,
                                fields,
                                Some(&resolved_name),
                                Some(variant.as_str()),
                            );
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
                    // WS-4 4c: a bare `Constructor` pattern with no
                    // enum-name (`Point { x, y }`) may name a registered
                    // *struct*/TypedObject, not an enum variant. Resolve
                    // `variant` against the schema registry; if it is a
                    // non-enum schema with a struct payload, route to the
                    // `Pattern::Object` struct-pattern codegen. The
                    // bare-enum-variant reject stays only for the genuine
                    // case (no schema, or an enum schema).
                    let resolved_name = self.resolve_type_name(variant);
                    let is_struct_schema = self
                        .type_tracker
                        .schema_registry()
                        .get(resolved_name.as_str())
                        .map(|s| !s.is_enum())
                        .unwrap_or(false);
                    if is_struct_schema {
                        if let PatternConstructorFields::Struct(field_pats) = fields {
                            return self.compile_match_binding_local(
                                &Pattern::Object(field_pats.clone()),
                                value_local,
                            );
                        }
                    }
                    Err(ShapeError::SemanticError {
                        message: "Bare enum variant patterns require type-resolved enum context. Generic fallback is disabled.".to_string(),
                        location: None,
                    })
                }
            },
        }
    }

    /// F5 (v0.3.3 strict-flip): after `match r { Ok(v) => … }` /
    /// `Some(v)` / `Err(e)` unwraps the scrutinee payload into `inner_local`,
    /// stamp `inner_local`'s tracked ConcreteType / element / type-name from
    /// the scrutinee's already-proven payload type. Without this the unwrapped
    /// binder is `unknown` and a downstream `v * 2` rejects as
    /// `unknown * int`.
    ///
    /// The payload type comes verbatim from `value_local`'s recorded
    /// `ConcreteType` (`Result(T, E)` / `Option(T)`) — no fabrication. When the
    /// scrutinee has no recorded concrete type (still generic / unannotated),
    /// nothing is stamped and the pre-existing behavior is preserved.
    pub(in crate::compiler) fn stamp_unwrapped_payload_local(
        &mut self,
        value_local: u16,
        inner_local: u16,
        variant: &str,
    ) {
        use shape_value::v2::ConcreteType;
        let Some(scrutinee_ct) = self
            .current_function_local_concrete_types
            .get(&value_local)
            .cloned()
        else {
            return;
        };
        let payload_ct = match (&scrutinee_ct, variant) {
            (ConcreteType::Result(ok, _), "Ok") => (**ok).clone(),
            (ConcreteType::Result(_, err), "Err") => (**err).clone(),
            (ConcreteType::Option(inner), "Some") => (**inner).clone(),
            _ => return,
        };
        self.stamp_local_from_concrete_type(inner_local, &payload_ct);
    }

    /// Stamp a local slot's tracked type info (ConcreteType + element/map
    /// side-tables + tracker type-name) from a known `ConcreteType`. Mirrors
    /// the stamps `finalize_empty_array_accumulator_kind` records for a
    /// promoted accumulator, so a downstream `xs[i]` / `.method()` / operator
    /// resolves exactly as for an annotated binding (ADR-006 §2.7.5).
    fn stamp_local_from_concrete_type(
        &mut self,
        local_idx: u16,
        ct: &shape_value::v2::ConcreteType,
    ) {
        use shape_value::v2::ConcreteType;
        self.current_function_local_concrete_types
            .insert(local_idx, ct.clone());
        match ct {
            ConcreteType::Array(elem) => {
                self.local_array_element_types
                    .insert(local_idx, (**elem).clone());
            }
            ConcreteType::HashMap(k, v) => {
                self.local_map_key_value_types
                    .insert(local_idx, ((**k).clone(), (**v).clone()));
            }
            _ => {}
        }
        if let Some(name) = concrete_type_tracker_name(ct) {
            self.set_local_type_info(local_idx, &name);
        }
    }

    /// Compile enum binding for TypedObject (optimized path)
    fn compile_typed_enum_binding(
        &mut self,
        value_local: u16,
        schema_id: u32,
        fields: &PatternConstructorFields,
        enum_name: Option<&str>,
        variant_name: Option<&str>,
    ) -> Result<()> {
        match fields {
            PatternConstructorFields::Unit => Ok(()),
            PatternConstructorFields::Tuple(patterns) => {
                // R8 W7: look up the per-position payload annotations
                // (parallel to the Struct arm's
                // `enum_struct_variant_fields` flow). Without this the
                // tuple-payload binders fall through to Unknown and
                // surface as "Cannot infer types for binary operation
                // `Add`: operand types are `string` and `unknown`" when
                // a downstream binop touches them.
                let variant_tuple_tys: Option<Vec<shape_ast::ast::TypeAnnotation>> =
                    match (enum_name, variant_name) {
                        (Some(en), Some(vn)) => self
                            .enum_tuple_variant_fields
                            .get(&(en.to_string(), vn.to_string()))
                            .cloned()
                            .or_else(|| {
                                en.rsplit("::").next().and_then(|bare| {
                                    self.enum_tuple_variant_fields
                                        .get(&(bare.to_string(), vn.to_string()))
                                        .cloned()
                                })
                            }),
                        _ => None,
                    };

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
                    // R8 W7: propagate the variant's positional payload
                    // type onto the temp local so the downstream
                    // Identifier/Typed binding inherits it via the
                    // existing source_info copy in those arms — same
                    // shape as the struct-arm variant_fields path.
                    if let Some(ref tys) = variant_tuple_tys {
                        if let Some(ann) = tys.get(idx) {
                            if let Some(tn) =
                                BytecodeCompiler::tracked_type_name_from_annotation(ann)
                            {
                                self.set_local_type_info(elem_local, &tn);
                            }
                        }
                    }
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(elem_local)),
                    ));
                    self.compile_match_binding_local(pat, elem_local)?;
                }
                Ok(())
            }
            PatternConstructorFields::Struct(patterns) => {
                // Sweep phase 3c.x: look up the (enum, variant) struct
                // field annotations so x and y in `m::E::V { x, y } => x + y`
                // get their declared int/number type instead of falling
                // through to Unknown. The schema only stores
                // `__payload_N: Any` for variant payloads; the named
                // field types live on the side in
                // `enum_struct_variant_fields`, populated by
                // `register_enum`.
                let variant_fields: Option<Vec<(String, shape_ast::ast::TypeAnnotation)>> =
                    match (enum_name, variant_name) {
                        (Some(en), Some(vn)) => self
                            .enum_struct_variant_fields
                            .get(&(en.to_string(), vn.to_string()))
                            .cloned()
                            .or_else(|| {
                                // Fall back to bare-name lookup (e.g. inside
                                // a `mod m` block, an `E::V` pattern may
                                // resolve `enum_name` differently than the
                                // qualified key).
                                en.rsplit("::").next().and_then(|bare| {
                                    self.enum_struct_variant_fields
                                        .get(&(bare.to_string(), vn.to_string()))
                                        .cloned()
                                })
                            }),
                        _ => None,
                    };

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
                    // Sweep phase 3c.x: propagate the variant struct
                    // field's declared type onto the temp local so the
                    // downstream Identifier/Typed binding inherits it
                    // via the existing source_info copy in those arms.
                    if let Some(ref vf) = variant_fields {
                        if let Some((_, ann)) = vf.get(idx) {
                            if let Some(tn) =
                                BytecodeCompiler::tracked_type_name_from_annotation(ann)
                            {
                                self.set_local_type_info(field_local, &tn);
                            }
                        }
                    }
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

    // ─── WS-4 4c: `match` struct-pattern classification ─────────────
    //
    // A bare `Constructor` pattern with no enum-name (`Point { x, y }`)
    // over a registered struct/TypedObject scrutinee must route to the
    // `Pattern::Object` codegen instead of being rejected as a bare
    // enum-variant pattern. Also covers 4c defect (i): a bare
    // struct-literal `match` scrutinee parses correctly.
    use crate::test_utils::eval;

    #[test]
    fn ws4_4c_match_struct_pattern_over_variable() {
        let result = eval(
            r#"
            type Point { x: int, y: int }
            let p = Point { x: 3, y: 4 }
            match p { Point { x, y } => x + y }
            "#,
        );
        assert_eq!(result.as_i64(), Some(7));
    }

    #[test]
    fn ws4_4c_match_struct_pattern_paren_scrutinee() {
        let result = eval(
            r#"
            type Point { x: int, y: int }
            match (Point { x: 3, y: 4 }) { Point { x, y } => x + y }
            "#,
        );
        assert_eq!(result.as_i64(), Some(7));
    }

    #[test]
    fn ws4_4c_match_struct_pattern_bare_struct_literal_scrutinee() {
        // 4c defect (i): the bare struct-literal scrutinee
        // `match Point { … } { … }` previously mis-parsed the
        // struct-literal body as the match body.
        let result = eval(
            r#"
            type Point { x: int, y: int }
            match Point { x: 3, y: 4 } { Point { x, y } => x + y }
            "#,
        );
        assert_eq!(result.as_i64(), Some(7));
    }

    #[test]
    fn ws4_4c_match_struct_pattern_in_function() {
        let result = eval(
            r#"
            type Point { x: int, y: int }
            fn f(p: Point) -> int { match p { Point { x, y } => x * y } }
            f(Point { x: 10, y: 20 })
            "#,
        );
        assert_eq!(result.as_i64(), Some(200));
    }

    #[test]
    fn ws4_4c_enum_match_still_works() {
        // The classification fix must not regress enum-variant matching:
        // a bare-name enum variant with no enum context still rejects.
        let result = eval(
            r#"
            enum E { A, B }
            let e = E::A
            match e { E::A => 1, E::B => 2 }
            "#,
        );
        assert_eq!(result.as_i64(), Some(1));
    }

    #[test]
    fn ws4_4c_bare_match_scrutinee_still_works() {
        // The `match_scrutinee_ident` fast-path must still handle a
        // bare-variable scrutinee with literal arms.
        let result = eval(
            r#"
            let v = 2
            match v { 1 => 10, 2 => 20, _ => 0 }
            "#,
        );
        assert_eq!(result.as_i64(), Some(20));
    }
}
