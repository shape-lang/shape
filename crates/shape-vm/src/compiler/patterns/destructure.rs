//! Destructure patterns - extracting values from compound structures

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::VariableTypeInfo;
use shape_ast::ast::{DecompositionBinding, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};

use crate::compiler::BytecodeCompiler;

impl BytecodeCompiler {
    fn schema_from_last_expr_type_info(&self) -> Option<u32> {
        self.last_expr_type_info
            .as_ref()
            .and_then(|info| info.schema_id)
    }

    fn resolve_object_destructure_schema(
        &mut self,
        fields: &[shape_ast::ast::ObjectPatternField],
    ) -> Option<u32> {
        if let Some(schema_id) = self.last_expr_schema {
            return Some(schema_id);
        }
        if let Some(schema_id) = self.schema_from_last_expr_type_info() {
            return Some(schema_id);
        }

        let explicit_fields: Vec<&str> = fields
            .iter()
            .filter_map(|field| match field.pattern {
                shape_ast::ast::DestructurePattern::Rest(_) => None,
                _ => Some(field.key.as_str()),
            })
            .collect();
        Some(
            self.type_tracker
                .register_inline_object_schema(&explicit_fields),
        )
    }

    fn resolve_decomposition_source_schema(
        &mut self,
        resolved_bindings: &[(Vec<String>, u32)],
    ) -> Option<u32> {
        if let Some(schema_id) = self.last_expr_schema {
            return Some(schema_id);
        }
        if let Some(schema_id) = self.schema_from_last_expr_type_info() {
            return Some(schema_id);
        }

        let mut ordered_fields: Vec<&str> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (fields, _) in resolved_bindings {
            for field in fields {
                if seen.insert(field.as_str()) {
                    ordered_fields.push(field.as_str());
                }
            }
        }
        Some(
            self.type_tracker
                .register_inline_object_schema(&ordered_fields),
        )
    }

    fn resolve_typed_field_operand_destructure(
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

    /// Compile destructuring pattern for value on stack
    /// Assumes value is already on the stack
    pub(in crate::compiler) fn compile_destructure_pattern(
        &mut self,
        pattern: &shape_ast::ast::DestructurePattern,
    ) -> Result<()> {
        use shape_ast::ast::DestructurePattern;

        match pattern {
            DestructurePattern::Identifier(name, _) => {
                // Simple case - store in local
                let local_idx = self.declare_local(name)?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(local_idx)),
                ));
                // Track schema for typed merge optimization
                if let Some(schema_id) = self.last_expr_schema {
                    self.type_tracker.set_local_type(
                        local_idx,
                        VariableTypeInfo::known(schema_id, format!("__typed_obj_{}", schema_id)),
                    );
                }
                Ok(())
            }

            DestructurePattern::Array(patterns) => {
                let value_local = self.declare_temp_local("__destructure_array_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit_destructure_type_check(
                    value_local,
                    "array",
                    "Cannot destructure non-array value as array",
                )?;

                for (index, pat) in patterns.iter().enumerate() {
                    if let DestructurePattern::Rest(inner) = pat {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        let idx_const = self.program.add_constant(Constant::Number(index as f64));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(idx_const)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        self.emit(Instruction::simple(OpCode::Length));
                        self.emit(Instruction::simple(OpCode::SliceAccess));
                        self.compile_destructure_pattern(inner)?;
                        break;
                    }

                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let idx_const = self.program.add_constant(Constant::Number(index as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(idx_const)),
                    ));
                    self.emit(Instruction::simple(OpCode::GetProp));
                    self.compile_destructure_pattern(pat)?;
                }

                Ok(())
            }

            DestructurePattern::Object(fields) => {
                let value_local = self.declare_temp_local("__destructure_object_")?;
                let object_schema = self.resolve_object_destructure_schema(fields);
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit_destructure_type_check(
                    value_local,
                    "object",
                    "Cannot destructure non-object value as object",
                )?;

                let mut rest_pattern: Option<&DestructurePattern> = None;
                let mut rest_excluded = Vec::new();

                let schema_id = object_schema.ok_or_else(|| ShapeError::SemanticError {
                    message: "Object destructuring requires a compile-time known schema. Runtime property lookup is disabled.".to_string(),
                    location: None,
                })?;

                for field in fields {
                    if let DestructurePattern::Rest(inner) = &field.pattern {
                        rest_pattern = Some(inner.as_ref());
                        continue;
                    }

                    rest_excluded.push(field.key.clone());
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let operand = self
                        .resolve_typed_field_operand_destructure(schema_id, &field.key)
                        .ok_or_else(|| ShapeError::SemanticError {
                            message: format!(
                                "Field '{}' is not declared in object schema for destructuring.",
                                field.key
                            ),
                            location: None,
                        })?;
                    self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    // Clear schema after field extraction — the extracted value is a
                    // scalar (number, string, etc.), not the parent TypedObject.
                    self.last_expr_schema = None;
                    self.last_expr_type_info = None;
                    self.compile_destructure_pattern(&field.pattern)?;
                }

                if let Some(rest) = rest_pattern {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit_object_rest(&rest_excluded, object_schema)?;
                    self.compile_destructure_pattern(rest)?;
                }

                Ok(())
            }

            DestructurePattern::Rest(_) => {
                // Rest patterns not yet supported in VM
                Err(ShapeError::RuntimeError {
                    message: "Rest pattern cannot be used at top level".to_string(),
                    location: None,
                })
            }
            DestructurePattern::Decomposition(bindings) => {
                // Decomposition extracts component types from intersection (A+B)
                // Splits the intersection value into separate objects by type
                let value_local = self.declare_temp_local("__decomposition_")?;
                let mut resolved_bindings = Vec::with_capacity(bindings.len());
                for binding in bindings {
                    let (fields, schema_id) = self.resolve_decomposition_binding(binding)?;
                    resolved_bindings.push((binding.name.clone(), fields, schema_id));
                }
                let source_schema_id = self
                    .resolve_decomposition_source_schema(
                        &resolved_bindings
                            .iter()
                            .map(|(_, fields, schema_id)| (fields.clone(), *schema_id))
                            .collect::<Vec<_>>(),
                    )
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: "Decomposition requires compile-time known source schema. Runtime property lookup is disabled.".to_string(),
                        location: None,
                    })?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));

                for (binding_name, fields, schema_id) in resolved_bindings {
                    for field_name in &fields {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        let operand = self
                            .resolve_typed_field_operand_destructure(source_schema_id, field_name)
                            .ok_or_else(|| ShapeError::SemanticError {
                                message: format!(
                                    "Field '{}' is not declared in decomposition source schema.",
                                    field_name
                                ),
                                location: None,
                            })?;
                        self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    }

                    self.emit(Instruction::new(
                        OpCode::NewTypedObject,
                        Some(Operand::TypedObjectAlloc {
                            schema_id: schema_id as u16,
                            field_count: fields.len() as u16,
                        }),
                    ));

                    let local_idx = self.declare_local(&binding_name)?;
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(local_idx)),
                    ));
                    let schema_name = self
                        .type_tracker
                        .schema_registry()
                        .get_by_id(schema_id)
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| format!("__typed_obj_{}", schema_id));
                    self.type_tracker
                        .set_local_type(local_idx, VariableTypeInfo::known(schema_id, schema_name));
                }
                Ok(())
            }
        }
    }

    pub(in crate::compiler) fn compile_destructure_pattern_global(
        &mut self,
        pattern: &shape_ast::ast::DestructurePattern,
    ) -> Result<()> {
        use shape_ast::ast::DestructurePattern;

        match pattern {
            DestructurePattern::Identifier(name, _) => {
                let binding_idx = self.get_or_create_module_binding(name);
                self.emit(Instruction::new(
                    OpCode::StoreModuleBinding,
                    Some(Operand::ModuleBinding(binding_idx)),
                ));
                // Track schema for typed merge optimization
                if let Some(schema_id) = self.last_expr_schema {
                    self.type_tracker.set_binding_type(
                        binding_idx,
                        VariableTypeInfo::known(schema_id, format!("__typed_obj_{}", schema_id)),
                    );
                }
                Ok(())
            }
            DestructurePattern::Array(patterns) => {
                let value_local = self.declare_temp_local("__destructure_array_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit_destructure_type_check(
                    value_local,
                    "array",
                    "Cannot destructure non-array value as array",
                )?;

                for (index, pat) in patterns.iter().enumerate() {
                    if let DestructurePattern::Rest(inner) = pat {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        let idx_const = self.program.add_constant(Constant::Number(index as f64));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(idx_const)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        self.emit(Instruction::simple(OpCode::Length));
                        self.emit(Instruction::simple(OpCode::SliceAccess));
                        self.compile_destructure_pattern_global(inner)?;
                        break;
                    }

                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let idx_const = self.program.add_constant(Constant::Number(index as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(idx_const)),
                    ));
                    self.emit(Instruction::simple(OpCode::GetProp));
                    self.compile_destructure_pattern_global(pat)?;
                }

                Ok(())
            }
            DestructurePattern::Object(fields) => {
                let value_local = self.declare_temp_local("__destructure_object_")?;
                let object_schema = self.resolve_object_destructure_schema(fields);
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit_destructure_type_check(
                    value_local,
                    "object",
                    "Cannot destructure non-object value as object",
                )?;

                let mut rest_pattern: Option<&DestructurePattern> = None;
                let mut rest_excluded = Vec::new();

                let schema_id = object_schema.ok_or_else(|| ShapeError::SemanticError {
                    message: "Object destructuring requires a compile-time known schema. Runtime property lookup is disabled.".to_string(),
                    location: None,
                })?;

                for field in fields {
                    if let DestructurePattern::Rest(inner) = &field.pattern {
                        rest_pattern = Some(inner.as_ref());
                        continue;
                    }

                    rest_excluded.push(field.key.clone());
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let operand = self
                        .resolve_typed_field_operand_destructure(schema_id, &field.key)
                        .ok_or_else(|| ShapeError::SemanticError {
                            message: format!(
                                "Field '{}' is not declared in object schema for destructuring.",
                                field.key
                            ),
                            location: None,
                        })?;
                    self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    // Clear schema after field extraction — the extracted value is a
                    // scalar (number, string, etc.), not the parent TypedObject.
                    self.last_expr_schema = None;
                    self.last_expr_type_info = None;
                    self.compile_destructure_pattern_global(&field.pattern)?;
                }

                if let Some(rest) = rest_pattern {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit_object_rest(&rest_excluded, object_schema)?;
                    self.compile_destructure_pattern_global(rest)?;
                }

                Ok(())
            }
            DestructurePattern::Rest(_) => Err(ShapeError::RuntimeError {
                message: "Rest pattern cannot be used at top level".to_string(),
                location: None,
            }),
            DestructurePattern::Decomposition(bindings) => {
                // Decomposition extracts component types from intersection (module_binding version)
                let value_local = self.declare_temp_local("__decomposition_")?;
                let mut resolved_bindings = Vec::with_capacity(bindings.len());
                for binding in bindings {
                    let (fields, schema_id) = self.resolve_decomposition_binding(binding)?;
                    resolved_bindings.push((binding.name.clone(), fields, schema_id));
                }
                let source_schema_id = self
                    .resolve_decomposition_source_schema(
                        &resolved_bindings
                            .iter()
                            .map(|(_, fields, schema_id)| (fields.clone(), *schema_id))
                            .collect::<Vec<_>>(),
                    )
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: "Decomposition requires compile-time known source schema. Runtime property lookup is disabled.".to_string(),
                        location: None,
                    })?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));

                for (binding_name, fields, schema_id) in resolved_bindings {
                    for field_name in &fields {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        let operand = self
                            .resolve_typed_field_operand_destructure(source_schema_id, field_name)
                            .ok_or_else(|| ShapeError::SemanticError {
                                message: format!(
                                    "Field '{}' is not declared in decomposition source schema.",
                                    field_name
                                ),
                                location: None,
                            })?;
                        self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    }

                    self.emit(Instruction::new(
                        OpCode::NewTypedObject,
                        Some(Operand::TypedObjectAlloc {
                            schema_id: schema_id as u16,
                            field_count: fields.len() as u16,
                        }),
                    ));

                    let binding_idx = self.get_or_create_module_binding(&binding_name);
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                    let schema_name = self
                        .type_tracker
                        .schema_registry()
                        .get_by_id(schema_id)
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| format!("__typed_obj_{}", schema_id));
                    self.type_tracker.set_binding_type(
                        binding_idx,
                        VariableTypeInfo::known(schema_id, schema_name),
                    );
                }
                Ok(())
            }
        }
    }

    pub(in crate::compiler) fn compile_destructure_assignment(
        &mut self,
        pattern: &shape_ast::ast::DestructurePattern,
    ) -> Result<()> {
        use shape_ast::ast::DestructurePattern;

        match pattern {
            DestructurePattern::Identifier(name, _) => self.emit_store_identifier(name),
            DestructurePattern::Array(patterns) => {
                let value_local = self.declare_temp_local("__assign_array_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit_destructure_type_check(
                    value_local,
                    "array",
                    "Cannot destructure non-array value as array",
                )?;

                for (index, pat) in patterns.iter().enumerate() {
                    if let DestructurePattern::Rest(inner) = pat {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        let idx_const = self.program.add_constant(Constant::Number(index as f64));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(idx_const)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        self.emit(Instruction::simple(OpCode::Length));
                        self.emit(Instruction::simple(OpCode::SliceAccess));
                        self.compile_destructure_assignment(inner)?;
                        break;
                    }

                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let idx_const = self.program.add_constant(Constant::Number(index as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(idx_const)),
                    ));
                    self.emit(Instruction::simple(OpCode::GetProp));
                    self.compile_destructure_assignment(pat)?;
                }

                Ok(())
            }
            DestructurePattern::Object(fields) => {
                let value_local = self.declare_temp_local("__assign_object_")?;
                let object_schema = self.last_expr_schema;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit_destructure_type_check(
                    value_local,
                    "object",
                    "Cannot destructure non-object value as object",
                )?;

                let mut rest_pattern: Option<&DestructurePattern> = None;
                let mut rest_excluded = Vec::new();

                let schema_id = object_schema.ok_or_else(|| ShapeError::SemanticError {
                    message: "Object destructuring assignment requires a compile-time known schema. Runtime property lookup is disabled.".to_string(),
                    location: None,
                })?;

                for field in fields {
                    if let DestructurePattern::Rest(inner) = &field.pattern {
                        rest_pattern = Some(inner.as_ref());
                        continue;
                    }

                    rest_excluded.push(field.key.clone());
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let operand = self.resolve_typed_field_operand_destructure(schema_id, &field.key).ok_or_else(|| ShapeError::SemanticError {
                        message: format!(
                            "Field '{}' is not declared in object schema for destructuring assignment.",
                            field.key
                        ),
                        location: None,
                    })?;
                    self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    self.compile_destructure_assignment(&field.pattern)?;
                }

                if let Some(rest) = rest_pattern {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit_object_rest(&rest_excluded, object_schema)?;
                    self.compile_destructure_assignment(rest)?;
                }

                Ok(())
            }
            DestructurePattern::Rest(_) => Err(ShapeError::RuntimeError {
                message: "Rest pattern cannot be used at top level".to_string(),
                location: None,
            }),
            DestructurePattern::Decomposition(bindings) => {
                // Decomposition extracts component types from intersection (assignment version)
                let value_local = self.declare_temp_local("__decomposition_")?;
                let source_schema_id = self.last_expr_schema.ok_or_else(|| {
                    ShapeError::SemanticError {
                        message: "Decomposition assignment requires compile-time known source schema. Runtime property lookup is disabled.".to_string(),
                        location: None,
                    }
                })?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));

                for binding in bindings {
                    let (fields, schema_id) = self.resolve_decomposition_binding(binding)?;

                    for field_name in &fields {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        let operand = self
                            .resolve_typed_field_operand_destructure(source_schema_id, field_name)
                            .ok_or_else(|| ShapeError::SemanticError {
                                message: format!(
                                    "Field '{}' is not declared in decomposition source schema.",
                                    field_name
                                ),
                                location: None,
                            })?;
                        self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    }

                    self.emit(Instruction::new(
                        OpCode::NewTypedObject,
                        Some(Operand::TypedObjectAlloc {
                            schema_id: schema_id as u16,
                            field_count: fields.len() as u16,
                        }),
                    ));

                    self.emit_store_identifier(&binding.name)?;
                }
                Ok(())
            }
        }
    }

    /// Resolve a decomposition binding's type annotation into field names and a schema ID.
    /// Handles both named types (e.g. `TypeA`) and inline object types (e.g. `{x, y}`).
    fn resolve_decomposition_binding(
        &mut self,
        binding: &DecompositionBinding,
    ) -> Result<(Vec<String>, u32)> {
        match &binding.type_annotation {
            TypeAnnotation::Object(obj_fields) => {
                // Inline object type: {x, y, z} or {x: int, y: string}
                let fields: Vec<String> = obj_fields.iter().map(|f| f.name.clone()).collect();
                let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
                let schema_id = self.type_tracker.register_inline_object_schema(&field_refs);
                Ok((fields, schema_id))
            }
            _ => {
                // Named type: look up from struct registry
                let type_name = binding.type_annotation.as_simple_name().ok_or_else(|| {
                    ShapeError::SemanticError {
                        message: "Decomposition binding requires a named type or object field set"
                            .to_string(),
                        location: Some(self.span_to_source_location(binding.span)),
                    }
                })?;

                let fields = self
                    .struct_types
                    .get(type_name)
                    .map(|(f, _)| f.clone())
                    .unwrap_or_default();

                let schema_id = self
                    .type_tracker
                    .schema_registry()
                    .get(type_name)
                    .map(|s| s.id)
                    .unwrap_or_else(|| {
                        let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
                        self.type_tracker.register_inline_object_schema(&field_refs)
                    });

                Ok((fields, schema_id))
            }
        }
    }
}
