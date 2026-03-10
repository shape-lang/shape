//! Property and index access expression compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::NumericType;
use shape_ast::ast::{DataIndex, Expr, Spanned, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_schema::FieldType;
use shape_runtime::type_system::{BuiltinTypes, Type};

use super::super::BytecodeCompiler;

/// Map a FieldType to a NumericType for typed opcode emission.
fn field_type_to_numeric(ft: &FieldType) -> Option<NumericType> {
    match ft {
        FieldType::I64 | FieldType::Timestamp => Some(NumericType::Int),
        FieldType::I8 => Some(NumericType::IntWidth(shape_ast::IntWidth::I8)),
        FieldType::U8 => Some(NumericType::IntWidth(shape_ast::IntWidth::U8)),
        FieldType::I16 => Some(NumericType::IntWidth(shape_ast::IntWidth::I16)),
        FieldType::U16 => Some(NumericType::IntWidth(shape_ast::IntWidth::U16)),
        FieldType::I32 => Some(NumericType::IntWidth(shape_ast::IntWidth::I32)),
        FieldType::U32 => Some(NumericType::IntWidth(shape_ast::IntWidth::U32)),
        FieldType::U64 => Some(NumericType::IntWidth(shape_ast::IntWidth::U64)),
        FieldType::F64 => Some(NumericType::Number),
        FieldType::Decimal => Some(NumericType::Decimal),
        _ => None,
    }
}

fn basic_name_to_numeric(name: &str) -> Option<NumericType> {
    if BuiltinTypes::is_integer_type_name(name) {
        return Some(NumericType::Int);
    }
    if BuiltinTypes::is_number_type_name(name) {
        return Some(NumericType::Number);
    }
    match name {
        "decimal" | "Decimal" => Some(NumericType::Decimal),
        _ => None,
    }
}

fn array_type_name_to_numeric(type_name: &str) -> Option<NumericType> {
    let inner = type_name
        .strip_prefix("Vec<")
        .and_then(|s| s.strip_suffix('>'))?;
    basic_name_to_numeric(inner.trim())
}

fn type_annotation_to_numeric(annotation: &TypeAnnotation) -> Option<NumericType> {
    match annotation {
        TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => {
            basic_name_to_numeric(name)
        }
        TypeAnnotation::Generic { name, args } if name == "Option" && args.len() == 1 => {
            type_annotation_to_numeric(&args[0])
        }
        _ => None,
    }
}

fn index_result_numeric_from_object_type(ty: &Type) -> Option<NumericType> {
    match ty {
        Type::Concrete(TypeAnnotation::Array(inner)) => type_annotation_to_numeric(inner),
        Type::Concrete(TypeAnnotation::Generic { name, args })
            if name == "Option" && args.len() == 1 =>
        {
            match &args[0] {
                TypeAnnotation::Array(elem) => type_annotation_to_numeric(elem),
                _ => None,
            }
        }
        _ => None,
    }
}

impl BytecodeCompiler {
    /// Compile a property access expression
    pub(super) fn compile_expr_property_access(
        &mut self,
        object: &Expr,
        property: &str,
        optional: bool,
    ) -> Result<()> {
        // Check for data[i].field pattern - emit GetDataField for direct column access
        if let Expr::DataRef(data_ref, _) = object {
            // Only optimize single index access with known column
            if let DataIndex::Single(idx) = &data_ref.index {
                if self.is_data_column(property) {
                    // Resolve column index at compile time
                    let col_idx = self.resolve_column_index(property)?;

                    // Push the row offset
                    let offset_const = self.program.add_constant(Constant::Number(*idx as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(offset_const)),
                    ));

                    // Emit GetDataField with compile-time column index
                    self.emit(Instruction::new(
                        OpCode::GetDataField,
                        Some(Operand::ColumnIndex(col_idx)),
                    ));
                    return Ok(());
                }
            }

            // Dynamic index with known column - still use GetDataField
            if let DataIndex::Expression(expr) = &data_ref.index {
                if self.is_data_column(property) {
                    let col_idx = self.resolve_column_index(property)?;

                    // Compile the index expression (pushes row offset)
                    self.compile_expr(expr)?;

                    // Emit GetDataField with compile-time column index
                    self.emit(Instruction::new(
                        OpCode::GetDataField,
                        Some(Operand::ColumnIndex(col_idx)),
                    ));
                    return Ok(());
                }
            }
        }

        // Check for RowView property access - emit typed column opcode
        if let Expr::Identifier(name, _) = object {
            if let Some(col_id) = self.try_resolve_row_view_column(name, property) {
                // Compile the object (pushes RowView onto stack)
                self.compile_expr(object)?;
                // Emit typed column load based on field type
                let opcode = self.row_view_field_opcode(name, property);
                self.emit(Instruction::new(
                    opcode,
                    Some(Operand::ColumnAccess { col_id }),
                ));
                self.last_expr_schema = None;
                self.last_expr_type_info = None;
                // Propagate numeric type from RowView field type
                self.last_expr_numeric_type =
                    self.resolve_row_view_field_numeric_type(name, property);
                return Ok(());
            }
            // If the variable IS a RowView but the field was NOT found → compile error
            if self.is_row_view_variable(name) {
                let field_names = self.get_row_view_field_names(name).unwrap_or_default();
                return Err(shape_ast::error::ShapeError::SemanticError {
                    message: format!(
                        "Field '{}' does not exist on Row<{}>. Available fields: {}",
                        property,
                        self.type_tracker
                            .get_local_type(self.resolve_local(name).unwrap_or(0))
                            .and_then(|i| i.type_name.clone())
                            .unwrap_or_else(|| "?".to_string()),
                        field_names.join(", "),
                    ),
                    location: None,
                });
            }
        }

        // Check for static-path comptime field access on type names (e.g. Currency.symbol).
        // The type name is not a variable, so we resolve the comptime field directly
        // without compiling the object expression.
        if let Expr::Identifier(type_name, _) = object {
            if let Some(comptime_value) = self
                .comptime_fields
                .get(type_name.as_str())
                .and_then(|m| m.get(property))
                .cloned()
            {
                let const_idx = if let Some(n) = comptime_value.as_number_coerce() {
                    self.program.add_constant(Constant::Number(n))
                } else if let Some(b) = comptime_value.as_bool() {
                    self.program.add_constant(Constant::Bool(b))
                } else if let Some(s) = comptime_value.as_str() {
                    self.program.add_constant(Constant::String(s.to_string()))
                } else {
                    self.program.add_constant(Constant::Null)
                };
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(const_idx)),
                ));
                self.last_expr_schema = None;
                self.last_expr_type_info = None;
                return Ok(());
            }
        }

        if !optional
            && let Some(place) = self.try_resolve_typed_field_place(object, property)
        {
            let label = format!("{}.{}", place.root_name, property);
            let source_loc = self.span_to_source_location(object.span());
            self.borrow_checker
                .check_read_allowed(place.borrow_key, Some(source_loc))
                .map_err(|err| Self::relabel_borrow_error(err, place.borrow_key, &label))?;

            let field_ref = self.declare_temp_local("__field_read_ref_")?;
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
            self.emit(Instruction::new(
                OpCode::DerefLoad,
                Some(Operand::Local(field_ref)),
            ));

            self.last_expr_schema = match &place.field_type_info {
                FieldType::Object(type_name) => self
                    .type_tracker
                    .schema_registry()
                    .get(type_name)
                    .map(|s| s.id),
                _ => None,
            };
            self.last_expr_type_info = None;
            self.last_expr_numeric_type = field_type_to_numeric(&place.field_type_info);
            return Ok(());
        }

        // Fall back to standard property access
        self.compile_expr(object)?;

        // Check for comptime field access — resolve to constant, zero runtime cost
        if let Some(schema_id) = self.last_expr_schema {
            let type_name = self
                .type_tracker
                .schema_registry()
                .get_by_id(schema_id)
                .map(|s| s.name.clone());

            if let Some(type_name) = type_name {
                // Clone the value out to release the borrow on self
                let comptime_value = self
                    .comptime_fields
                    .get(&type_name)
                    .and_then(|m| m.get(property))
                    .cloned();

                if let Some(value) = comptime_value {
                    // Pop the object — we don't need it for a comptime field
                    self.emit(Instruction::simple(OpCode::Pop));
                    // Push the constant value directly from ValueWord
                    let const_idx = if let Some(n) = value.as_number_coerce() {
                        self.program.add_constant(Constant::Number(n))
                    } else if let Some(b) = value.as_bool() {
                        self.program.add_constant(Constant::Bool(b))
                    } else if let Some(s) = value.as_str() {
                        self.program.add_constant(Constant::String(s.to_string()))
                    } else {
                        self.program.add_constant(Constant::Null)
                    };
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(const_idx)),
                    ));
                    self.last_expr_schema = None;
                    self.last_expr_type_info = None;
                    return Ok(());
                }
            }
        }

        let (typed_field, field_numeric_type, field_type_info) =
            if let Some(schema_id) = self.last_expr_schema {
                if schema_id > u16::MAX as u32 {
                    (None, None, None)
                } else {
                    self.type_tracker
                        .schema_registry()
                        .get_by_id(schema_id)
                        .and_then(|schema| {
                            schema.get_field(property).and_then(|field| {
                                if field.offset <= u16::MAX as usize {
                                    let numeric = field_type_to_numeric(&field.field_type);
                                    let ft = field.field_type.clone();
                                    Some((
                                        Some(Operand::TypedField {
                                            type_id: schema_id as u16,
                                            field_idx: field.index as u16,
                                            field_type_tag: field_type_to_tag(&field.field_type),
                                        }),
                                        numeric,
                                        Some(ft),
                                    ))
                                } else {
                                    None
                                }
                            })
                        })
                        .unwrap_or((None, None, None))
                }
            } else {
                (None, None, None)
            };

        let _unresolved_property_error = || ShapeError::SemanticError {
            message: format!(
                "Property '{}' must resolve at compile time. Generic runtime property lookup is disabled.",
                property
            ),
            location: None,
        };

        if optional {
            self.emit(Instruction::simple(OpCode::Dup));
            self.emit(Instruction::simple(OpCode::PushNull));
            self.emit(Instruction::simple(OpCode::Eq));
            let null_jump = self.emit_jump(OpCode::JumpIfTrue, 0);

            self.emit(Instruction::simple(OpCode::Dup));
            self.emit_unit();
            self.emit(Instruction::simple(OpCode::Eq));
            let unit_jump = self.emit_jump(OpCode::JumpIfTrue, 0);

            if let Some(operand) = typed_field {
                self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
            } else if property == "length" {
                self.emit(Instruction::simple(OpCode::Length));
            } else {
                let prop_const = self
                    .program
                    .add_constant(Constant::String(property.to_string()));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(prop_const)),
                ));
                self.emit(Instruction::simple(OpCode::GetProp));
            }

            let end_jump = self.emit_jump(OpCode::Jump, 0);

            self.patch_jump(null_jump);
            self.patch_jump(unit_jump);
            self.emit(Instruction::simple(OpCode::Pop));
            self.emit(Instruction::simple(OpCode::PushNull));
            self.patch_jump(end_jump);
        } else if let Some(operand) = typed_field {
            self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
        } else if property == "length" {
            self.emit(Instruction::simple(OpCode::Length));
        } else {
            let prop_const = self
                .program
                .add_constant(Constant::String(property.to_string()));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(prop_const)),
            ));
            self.emit(Instruction::simple(OpCode::GetProp));
        }
        // Propagate nested object schema for chained property access (e.g. cfg.server.host).
        // If the field type is Object(type_name), resolve its schema ID so subsequent
        // property accesses can emit GetFieldTyped.
        self.last_expr_schema = match &field_type_info {
            Some(FieldType::Object(type_name)) => self
                .type_tracker
                .schema_registry()
                .get(type_name)
                .map(|s| s.id),
            _ => None,
        };
        self.last_expr_type_info = None;
        // Propagate numeric type from field type for typed opcode emission
        self.last_expr_numeric_type = field_numeric_type;
        Ok(())
    }

    /// Compile an index access expression
    pub(super) fn compile_expr_index_access(
        &mut self,
        object: &Expr,
        index: &Expr,
        end_index: &Option<Box<Expr>>,
    ) -> Result<()> {
        let tracked_numeric = if let Expr::Identifier(name, _) = object {
            if let Some(local_idx) = self.resolve_local(name) {
                self.type_tracker
                    .get_local_type(local_idx)
                    .and_then(|info| info.type_name.as_deref())
                    .and_then(array_type_name_to_numeric)
            } else {
                let scoped_name = self
                    .resolve_scoped_module_binding_name(name)
                    .unwrap_or_else(|| name.to_string());
                self.module_bindings
                    .get(&scoped_name)
                    .and_then(|binding_idx| self.type_tracker.get_binding_type(*binding_idx))
                    .and_then(|info| info.type_name.as_deref())
                    .and_then(array_type_name_to_numeric)
            }
        } else {
            None
        };

        let inferred_numeric = if end_index.is_none() {
            tracked_numeric.or_else(|| {
                self.infer_expr_type(object)
                    .ok()
                    .and_then(|ty| index_result_numeric_from_object_type(&ty))
            })
        } else {
            None
        };
        self.compile_expr(object)?;
        self.compile_expr(index)?;
        if let Some(end) = end_index {
            // Slice access: array[start:end]
            self.compile_expr(end)?;
            self.emit(Instruction::simple(OpCode::SliceAccess));
        } else {
            // Single index access
            self.emit(Instruction::simple(OpCode::GetProp));
        }
        // Index access result is typically not a TypedObject
        self.last_expr_schema = None;
        self.last_expr_type_info = None;
        self.last_expr_numeric_type = inferred_numeric;
        Ok(())
    }
}
