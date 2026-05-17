//! Property and index access expression compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::NumericType;
use shape_ast::ast::{DataIndex, Expr, Spanned, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_schema::FieldType;
use shape_runtime::type_system::{BuiltinTypes, Type};

use shape_value::v2::struct_layout::FieldKind;

use super::super::BytecodeCompiler;

/// Classification of a local-slot receiver for typed `.length` emission.
enum TypedLengthLocal {
    /// Receiver is a typed array — emit `ArrayLenTyped(slot)`
    Array(u16),
    /// Receiver is a typed HashMap — emit `MapLenTyped(slot)`
    Map(u16),
    /// Receiver is a string — emit `StringLenTyped(slot)`
    String(u16),
}

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
        TypeAnnotation::Basic(name) => basic_name_to_numeric(name),
        TypeAnnotation::Reference(name) => basic_name_to_numeric(name),
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
        if let Expr::Identifier(name, span) = object
            && self.is_module_namespace_name(name)
            && self.resolve_local(name).is_none()
            && !self.mutable_closure_captures.contains_key(name.as_str())
        {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Module namespace access must use `::`. Replace `{}.{}` with an explicit import or `{}::...` call.",
                    name, property, name
                ),
                location: Some(self.span_to_source_location(*span)),
            });
        }

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
            if self
                .comptime_fields
                .get(type_name.as_str())
                .and_then(|m| m.get(property))
                .is_some()
            {
                // SURFACE: the kinded `KindedSlot → Constant` projection
                // for comptime field reads lives in phase-2c (ADR-006
                // §2.4). The carrier-tier `comptime_fields` registry is
                // already `HashMap<String, HashMap<String, KindedSlot>>`,
                // but the producer side that bakes comptime defaults into
                // it is dormant (see `statements.rs:2450-2512` —
                // recognised-literal arms are validated but never stored),
                // so this branch is currently unreachable in real
                // programs. Returning a structured semantic error rather
                // than a panic keeps the surface honest when a future
                // phase-2c commit wires the producer side but lands ahead
                // of the projector. Tracked as `c3-expr-lowering-misc`
                // per playbook §3 (Wave 2.5).
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "comptime field access '{}.{}' is dormant pending the phase-2c \
                         KindedSlot-to-Constant projection rebuild (ADR-006 §2.4 / §2.7.4)",
                        type_name, property
                    ),
                    location: Some(self.span_to_source_location(object.span())),
                });
            }
        }

        if !optional && let Some(place) = self.try_resolve_typed_field_place(object, property) {
            let label = format!("{}.{}", place.root_name, property);
            let source_loc = self.span_to_source_location(object.span());
            self.check_read_allowed_in_current_context(place.borrow_key, Some(source_loc))
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

        // v2 Phase 3.1 (Agent 3): typed-array `length` fast path.
        //
        // Resolve the receiver as a tracked typed array BEFORE compiling
        // the object expression (compile_expr may overwrite tracker state).
        // We only act on it for the `length` property below; other property
        // names continue down the legacy path. The receiver is still
        // compiled normally so the array pointer ends up on the stack —
        // `TypedArrayLen` pops the pointer just like the legacy `Length`
        // opcode does, so the only change is the opcode byte.
        let typed_array_for_length = if property == "length" {
            self.resolve_receiver_typed_array_kind(object)
        } else {
            None
        };

        // Typed collection `.length` local-slot fast path.
        //
        // When the receiver is an identifier in a local slot with a proven
        // collection type, emit the local-slot-based length opcode which
        // reads the receiver directly from the slot without pushing it
        // onto the stack.
        if property == "length" {
            if let Some(local) = self.try_resolve_typed_length_local(object) {
                match local {
                    TypedLengthLocal::Array(slot) => {
                        self.emit(Instruction::new(
                            OpCode::ArrayLenTyped,
                            Some(Operand::Local(slot)),
                        ));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.last_expr_numeric_type = Some(NumericType::Int);
                        return Ok(());
                    }
                    TypedLengthLocal::Map(slot) => {
                        self.emit(Instruction::new(
                            OpCode::MapLenTyped,
                            Some(Operand::Local(slot)),
                        ));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.last_expr_numeric_type = Some(NumericType::Int);
                        return Ok(());
                    }
                    TypedLengthLocal::String(slot) => {
                        self.emit(Instruction::new(
                            OpCode::StringLenTyped,
                            Some(Operand::Local(slot)),
                        ));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.last_expr_numeric_type = Some(NumericType::Int);
                        return Ok(());
                    }
                }
            }
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

                if comptime_value.is_some() {
                    // SURFACE: same boundary as the static-path branch
                    // above. The kinded `KindedSlot → Constant`
                    // projection for comptime field reads lives in
                    // phase-2c (ADR-006 §2.4 / §2.7.4); the producer side
                    // (`statements.rs:2450-2512`) is dormant so this
                    // branch is currently unreachable. Tracked as
                    // `c3-expr-lowering-misc` per playbook §3.
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "comptime field access '{}.{}' (via schema lookup) is dormant \
                             pending the phase-2c KindedSlot-to-Constant projection rebuild \
                             (ADR-006 §2.4 / §2.7.4)",
                            type_name, property
                        ),
                        location: Some(self.span_to_source_location(object.span())),
                    });
                }
            }
        }

        // Try v2 typed field access first (direct byte-offset load), then fall back to v1.
        let (typed_field, field_numeric_type, field_type_info, v2_load_opcode, v2_field_offset) =
            if let Some(schema_id) = self.last_expr_schema {
                if schema_id > u16::MAX as u32 {
                    (None, None, None, None, None)
                } else {
                    // Check for v2 StructLayout first
                    let v2_info = self
                        .type_tracker
                        .get_v2_layout(schema_id)
                        .and_then(|layout| {
                            // Find the field index by name in the layout
                            let field_idx = layout
                                .fields
                                .iter()
                                .position(|f| f.name == property)?;
                            let byte_offset = layout.field_offset(field_idx);
                            let field_kind = layout.field_kind(field_idx);
                            // Map FieldKind to v2 load opcode
                            let opcode = match field_kind {
                                FieldKind::F64 => OpCode::FieldLoadF64,
                                FieldKind::I64 | FieldKind::U64 => OpCode::FieldLoadI64,
                                FieldKind::I32 | FieldKind::U32 => OpCode::FieldLoadI32,
                                FieldKind::Bool => OpCode::FieldLoadBool,
                                FieldKind::Ptr => OpCode::FieldLoadPtr,
                                // Smaller types don't have dedicated v2 opcodes yet
                                _ => return None,
                            };
                            if byte_offset <= u16::MAX as usize {
                                Some((opcode, byte_offset as u16))
                            } else {
                                None
                            }
                        });

                    let schema_result = self
                        .type_tracker
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
                        .unwrap_or((None, None, None));

                    (
                        schema_result.0,
                        schema_result.1,
                        schema_result.2,
                        v2_info.map(|(op, _)| op),
                        v2_info.map(|(_, off)| off),
                    )
                }
            } else {
                (None, None, None, None, None)
            };

        let _unresolved_property_error = || ShapeError::SemanticError {
            message: format!(
                "Property '{}' must resolve at compile time. Generic runtime property lookup is disabled.",
                property
            ),
            location: None,
        };

        if optional {
            // Stage 2.6.5.2: a single typed IsNull check covers both the
            // None and Unit absence sentinels (the original optional
            // chaining desugar checked them separately). Two structurally
            // independent IsNull checks are kept here so that the
            // null_jump and unit_jump patch points stay distinct for the
            // surrounding control-flow code.
            self.emit(Instruction::simple(OpCode::Dup));
            self.emit(Instruction::simple(OpCode::IsNull));
            let null_jump = self.emit_jump(OpCode::JumpIfTrue, 0);

            self.emit(Instruction::simple(OpCode::Dup));
            self.emit(Instruction::simple(OpCode::IsNull));
            let unit_jump = self.emit_jump(OpCode::JumpIfTrue, 0);

            if let (Some(opcode), Some(offset)) = (v2_load_opcode, v2_field_offset) {
                self.emit(Instruction::new(opcode, Some(Operand::FieldOffset(offset))));
            } else if let Some(operand) = typed_field {
                self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
            } else if property == "length" {
                if typed_array_for_length.is_some() {
                    self.emit(Instruction::simple(OpCode::TypedArrayLen));
                } else {
                    self.emit(Instruction::simple(OpCode::Length));
                }
            } else {
                let prop_const = self
                    .program
                    .add_constant(Constant::String(property.to_string()));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(prop_const)),
                ));
                self.emit(Instruction::simple(OpCode::GetProp));
                self.record_get_prop_native_kind(field_type_info.as_ref());
            }

            let end_jump = self.emit_jump(OpCode::Jump, 0);

            self.patch_jump(null_jump);
            self.patch_jump(unit_jump);
            self.emit(Instruction::simple(OpCode::Pop));
            self.emit(Instruction::simple(OpCode::PushNull));
            self.patch_jump(end_jump);
        } else if let (Some(opcode), Some(offset)) = (v2_load_opcode, v2_field_offset) {
            self.emit(Instruction::new(opcode, Some(Operand::FieldOffset(offset))));
        } else if let Some(operand) = typed_field {
            self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
        } else if property == "length" {
            if typed_array_for_length.is_some() {
                self.emit(Instruction::simple(OpCode::TypedArrayLen));
            } else {
                self.emit(Instruction::simple(OpCode::Length));
            }
        } else {
            let prop_const = self
                .program
                .add_constant(Constant::String(property.to_string()));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(prop_const)),
            ));
            self.emit(Instruction::simple(OpCode::GetProp));
            self.record_get_prop_native_kind(field_type_info.as_ref());
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

        // v2 Phase 3.1 (Agent 3): typed-array fast path for `arr[i]`.
        // Resolve the receiver kind BEFORE compiling the object —
        // compile_expr may overwrite tracker state. Falls through to the
        // legacy `GetProp` path for non-Identifier receivers, slices,
        // untracked arrays, and any element type without a typed kind.
        let typed_kind = if end_index.is_none() {
            self.resolve_receiver_typed_array_kind(object)
        } else {
            None
        };

        // Local-slot-based typed-array index access fast path.
        //
        // When the receiver is an identifier in a local slot with a proven
        // typed-array kind (i64 or f64), emit `GetElemI64`/`GetElemF64`
        // with the local slot as operand. This avoids pushing the array
        // pointer onto the stack before the index.
        if end_index.is_none() {
            if let Some((slot, elem_opcode)) = self.try_resolve_typed_elem_get(object) {
                self.compile_expr(index)?;
                self.emit(Instruction::new(
                    elem_opcode,
                    Some(Operand::Local(slot)),
                ));
                self.last_expr_schema = None;
                self.last_expr_type_info = None;
                self.last_expr_numeric_type = inferred_numeric;
                return Ok(());
            }
        }

        // W1.11 (v0.3 R2): user-type `Index` trait dispatch for `c[k]`.
        //
        // After the built-in typed-array fast paths fail (typed_kind is
        // None and try_resolve_typed_elem_get returned None), check if the
        // receiver's type implements the `Index` trait. If so, emit
        // `CallMethod("index", arg_count=1)` instead of falling through to
        // the generic `GetProp` path. This is the index-access analog of
        // the binary-op trait dispatch at `binary_ops.rs:71-85`. Sibling
        // of `IndexMut` dispatch in `assignment.rs` for `c[k] = v`.
        //
        // Resolve the trait BEFORE compiling the object — `compile_expr`
        // may overwrite tracker state used by `infer_expr_type`.
        if end_index.is_none() && typed_kind.is_none() {
            if self.receiver_type_implements_trait(object, "Index") {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                emit_index_trait_call(self, "index", 1);
                self.last_expr_schema = None;
                self.last_expr_type_info = None;
                self.last_expr_numeric_type = None;
                return Ok(());
            }
        }

        self.compile_expr(object)?;
        self.compile_expr(index)?;
        if let Some(end) = end_index {
            // Slice access: array[start:end]
            self.compile_expr(end)?;
            self.emit(Instruction::simple(OpCode::SliceAccess));
        } else if let Some(kind) = typed_kind {
            // v2 Phase 3.1: typed array element load.
            self.emit(Instruction::simple(kind.get_opcode()));
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

    /// Try to resolve a receiver expression to a local-slot-based typed
    /// length opcode target.
    fn try_resolve_typed_length_local(&mut self, object: &Expr) -> Option<TypedLengthLocal> {
        let name = match object {
            Expr::Identifier(name, _) => name,
            _ => return None,
        };
        let local_idx = self.resolve_local(name)?;

        // Typed array (v2)
        if self.v2_typed_array_locals.contains_key(&local_idx) {
            return Some(TypedLengthLocal::Array(local_idx));
        }
        // Typed HashMap (v2)
        if self.v2_typed_map_locals.contains_key(&local_idx) {
            return Some(TypedLengthLocal::Map(local_idx));
        }
        // String (non-param locals with confirmed type name)
        if !self.param_locals.contains(&local_idx) {
            let is_string = self
                .type_tracker
                .get_local_type(local_idx)
                .and_then(|info| info.type_name.as_deref().map(|n| n == "string" || n == "String"))
                .unwrap_or(false);
            if is_string {
                return Some(TypedLengthLocal::String(local_idx));
            }
        }
        None
    }

    /// Try to resolve a receiver expression to a local-slot-based typed
    /// element get opcode. Returns `Some((slot, opcode))` for i64/f64
    /// typed arrays.
    fn try_resolve_typed_elem_get(&self, object: &Expr) -> Option<(u16, OpCode)> {
        let name = match object {
            Expr::Identifier(name, _) => name,
            _ => return None,
        };
        let local_idx = self.resolve_local(name)?;
        let kind = self.v2_typed_array_locals.get(&local_idx)?;
        match kind {
            crate::compiler::v2_typed_emission::TypedArrayKind::I64 => {
                Some((local_idx, OpCode::GetElemI64))
            }
            crate::compiler::v2_typed_emission::TypedArrayKind::F64 => {
                Some((local_idx, OpCode::GetElemF64))
            }
            _ => None,
        }
    }

    /// W1.11: Check whether the receiver `object` has a type that implements
    /// `trait_name` (e.g. `"Index"` or `"IndexMut"`).
    ///
    /// Schema-first lookup uses the tracker's last-expr schema if the
    /// receiver is an identifier; falls back to `infer_expr_type` +
    /// `type_display_name`. Mirrors `try_emit_trait_dispatch` at
    /// `binary_ops.rs:71-85` for the index-access dispatch path.
    pub(super) fn receiver_type_implements_trait(
        &mut self,
        object: &Expr,
        trait_name: &str,
    ) -> bool {
        // Schema-based check: if the receiver is a known identifier with a
        // recorded TypedObject schema, look up by schema name.
        let schema_type_name = if let Expr::Identifier(name, _) = object {
            self.tracker_type_name_for_identifier(name)
        } else {
            None
        };
        if let Some(type_name) = schema_type_name {
            if self
                .type_inference
                .env
                .type_implements_trait(&type_name, trait_name)
            {
                return true;
            }
        }
        // Inference-based fallback for non-Identifier receivers or
        // receivers without a tracker entry.
        if let Ok(ty) = self.infer_expr_type(object) {
            let name = super::numeric_ops::type_display_name(&ty);
            if self
                .type_inference
                .env
                .type_implements_trait(&name, trait_name)
            {
                return true;
            }
        }
        false
    }
}

/// W1.11: Emit a `CallMethod` instruction targeting an `Index`/`IndexMut`
/// trait method (e.g. `Cache::index`, `Cache::index_set`). All operands
/// must already be on the stack: receiver first, then the key (and
/// optionally the value for `index_set`). Mirrors
/// `emit_operator_trait_call` at `binary_ops.rs:90-104`.
pub(super) fn emit_index_trait_call(
    compiler: &mut BytecodeCompiler,
    method_name: &str,
    arg_count: u16,
) {
    let method_id = shape_value::MethodId::from_name(method_name);
    let string_id = compiler.program.add_string(method_name.to_string());
    compiler.emit(Instruction::new(
        OpCode::CallMethod,
        Some(Operand::TypedMethodCall {
            method_id: method_id.0,
            arg_count,
            string_id,
            receiver_type_tag: 0xFF,
        }),
    ));
}
