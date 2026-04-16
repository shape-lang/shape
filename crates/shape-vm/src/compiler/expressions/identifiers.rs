//! Identifier expression compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use shape_ast::ast::Span;
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_system::suggestions::suggest_variable;

use crate::type_tracking::{BindingStorageClass, NumericType, StorageHint, VariableKind};

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    pub(in crate::compiler) fn compile_expr_identifier_preserving_refs(
        &mut self,
        name: &str,
        span: Span,
    ) -> Result<()> {
        if let Some(local_idx) = self.resolve_local(name) {
            if self.ref_locals.contains(&local_idx) {
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(local_idx)),
                ));
                let mode = if self.exclusive_ref_locals.contains(&local_idx) {
                    crate::compiler::BorrowMode::Exclusive
                } else {
                    crate::compiler::BorrowMode::Shared
                };
                self.set_last_expr_reference_result(mode, true);
                return Ok(());
            }
            if self.reference_value_locals.contains(&local_idx) {
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(local_idx)),
                ));
                let mode = if self.exclusive_reference_value_locals.contains(&local_idx) {
                    crate::compiler::BorrowMode::Exclusive
                } else {
                    crate::compiler::BorrowMode::Shared
                };
                self.set_last_expr_reference_result(mode, true);
                return Ok(());
            }
        } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
            let binding_idx = *self.module_bindings.get(&scoped_name).ok_or_else(|| {
                ShapeError::RuntimeError {
                    message: self.undefined_variable_message(name),
                    location: Some(self.span_to_source_location(span)),
                }
            })?;
            if self.reference_value_module_bindings.contains(&binding_idx) {
                self.emit(Instruction::new(
                    OpCode::LoadModuleBinding,
                    Some(Operand::ModuleBinding(binding_idx)),
                ));
                let mode = if self
                    .exclusive_reference_value_module_bindings
                    .contains(&binding_idx)
                {
                    crate::compiler::BorrowMode::Exclusive
                } else {
                    crate::compiler::BorrowMode::Shared
                };
                self.set_last_expr_reference_result(mode, true);
                return Ok(());
            }
        }

        let result = self.compile_expr_identifier(name, span);
        if result.is_ok() {
            self.clear_last_expr_reference_result();
        }
        result
    }

    /// Map a storage hint to a numeric type (if applicable).
    /// Width-specific hints (Int8, UInt16, etc.) → IntWidth(w);
    /// default Int64 → Int; Float64 → Number.
    pub(in crate::compiler) fn storage_hint_to_numeric_type(
        hint: StorageHint,
    ) -> Option<NumericType> {
        use shape_ast::IntWidth;
        match hint {
            StorageHint::Int8 | StorageHint::NullableInt8 => {
                Some(NumericType::IntWidth(IntWidth::I8))
            }
            StorageHint::UInt8 | StorageHint::NullableUInt8 => {
                Some(NumericType::IntWidth(IntWidth::U8))
            }
            StorageHint::Int16 | StorageHint::NullableInt16 => {
                Some(NumericType::IntWidth(IntWidth::I16))
            }
            StorageHint::UInt16 | StorageHint::NullableUInt16 => {
                Some(NumericType::IntWidth(IntWidth::U16))
            }
            StorageHint::Int32 | StorageHint::NullableInt32 => {
                Some(NumericType::IntWidth(IntWidth::I32))
            }
            StorageHint::UInt32 | StorageHint::NullableUInt32 => {
                Some(NumericType::IntWidth(IntWidth::U32))
            }
            StorageHint::UInt64 | StorageHint::NullableUInt64 => {
                Some(NumericType::IntWidth(IntWidth::U64))
            }
            _ if hint.is_default_int_family() => Some(NumericType::Int),
            _ if hint.is_float_family() => Some(NumericType::Number),
            _ => None,
        }
    }

    /// Compile an identifier (variable or function reference)
    pub(in crate::compiler) fn compile_expr_identifier(
        &mut self,
        name: &str,
        span: Span,
    ) -> Result<()> {
        if name == "__comptime__" && !self.allow_internal_comptime_namespace {
            return Err(ShapeError::SemanticError {
                message: "`__comptime__` is an internal compiler namespace and is not accessible from source code".to_string(),
                location: Some(self.span_to_source_location(span)),
            });
        }
        // Mutable closure captures: emit LoadClosure to read from the shared upvalue
        if let Some(&upvalue_idx) = self.mutable_closure_captures.get(name) {
            self.emit(Instruction::new(
                OpCode::LoadClosure,
                Some(Operand::Local(upvalue_idx)),
            ));
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.last_expr_numeric_type = None;
            return Ok(());
        }
        if let Some(local_idx) = self.resolve_local(name) {
            if self.ref_locals.contains(&local_idx) {
                // Reference parameter: dereference to get the target value
                self.emit(Instruction::new(
                    OpCode::DerefLoad,
                    Some(Operand::Local(local_idx)),
                ));
            } else if self.reference_value_locals.contains(&local_idx) {
                self.emit(Instruction::new(
                    OpCode::DerefLoad,
                    Some(Operand::Local(local_idx)),
                ));
            } else {
                let source_loc = self.span_to_source_location(span);
                self.check_read_allowed_in_current_context(
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

                // Storage-plan–aware load decision
                // ─────────────────────────────────
                // The MIR storage planner assigns each binding a BindingStorageClass:
                //   Direct    → LoadLocal / LoadLocalTrusted (no indirection)
                //   Deferred  → same as Direct (plan not yet resolved)
                //   UniqueHeap→ BoxLocal + Arc<RwLock<>> (SharedCell), read via LoadClosure
                //   SharedCow → BoxLocal + Arc<RwLock<>> (SharedCell), read via LoadClosure
                //   Reference → DerefLoad / DerefStore (handled above)
                //
                // Consult the MIR storage plan first (authoritative when available),
                // then fall back to type-tracker semantics for non-function contexts.
                let storage_class = self.mir_storage_class_for_slot(local_idx).or_else(|| {
                    self.type_tracker
                        .get_local_binding_semantics(local_idx)
                        .map(|s| s.storage_class)
                });

                if self.boxed_locals.contains(name)
                    && matches!(
                        storage_class,
                        Some(BindingStorageClass::UniqueHeap | BindingStorageClass::SharedCow)
                    )
                {
                    // The variable has been boxed into a SharedCell by a prior
                    // closure capture — read through the cell.
                    self.emit(Instruction::new(
                        OpCode::LoadClosure,
                        Some(Operand::Local(local_idx)),
                    ));
                } else {
                    // Upgrade to LoadLocalTrusted when the slot has a known
                    // *primitive* type AND is immutable. We only upgrade for
                    // immutable let-bindings with int/float/bool slots to avoid
                    // breaking SharedCell, heap-type, or ref-mutated semantics.
                    if self.immutable_locals.contains(&local_idx)
                        && self
                            .type_tracker
                            .get_local_type(local_idx)
                            .map(|info| {
                                matches!(
                                    info.storage_hint,
                                    StorageHint::Int64 | StorageHint::Float64 | StorageHint::Bool
                                )
                            })
                            .unwrap_or(false)
                    {
                        self.emit(Instruction::new(
                            OpCode::LoadLocalTrusted,
                            Some(Operand::Local(local_idx)),
                        ));
                    } else {
                        // Ownership-aware load: consults MIR borrow analysis
                        // to emit LoadLocalMove / LoadLocalClone when the
                        // decision is available; falls back to plain LoadLocal
                        // for Copy types or when no MIR info exists.
                        self.emit_load_local_owned(local_idx, &span);
                    }
                }
            }
            // Track schema for typed merge optimization
            let local_type = self.type_tracker.get_local_type(local_idx).cloned();
            self.last_expr_schema = local_type.as_ref().and_then(|info| {
                if matches!(info.kind, VariableKind::Value) {
                    info.schema_id
                } else {
                    None
                }
            });
            self.last_expr_type_info = local_type;
            // Track numeric type for typed opcode emission
            self.last_expr_numeric_type = self
                .type_tracker
                .get_local_type(local_idx)
                .and_then(|info| Self::storage_hint_to_numeric_type(info.storage_hint));
        } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
            let binding_idx = *self.module_bindings.get(&scoped_name).ok_or_else(|| {
                ShapeError::RuntimeError {
                    message: self.undefined_variable_message(name),
                    location: Some(self.span_to_source_location(span)),
                }
            })?;
            let source_loc = self.span_to_source_location(span);
            self.check_read_allowed_in_current_context(
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
            if self.reference_value_module_bindings.contains(&binding_idx) {
                let temp = self.declare_temp_local("__module_binding_ref_read_")?;
                self.emit(Instruction::new(
                    OpCode::LoadModuleBinding,
                    Some(Operand::ModuleBinding(binding_idx)),
                ));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(temp)),
                ));
                self.emit(Instruction::new(
                    OpCode::DerefLoad,
                    Some(Operand::Local(temp)),
                ));
            } else {
                self.emit(Instruction::new(
                    OpCode::LoadModuleBinding,
                    Some(Operand::ModuleBinding(binding_idx)),
                ));
            }
            // Track schema for typed merge optimization
            let binding_type = self.type_tracker.get_binding_type(binding_idx).cloned();
            self.last_expr_schema = binding_type.as_ref().and_then(|info| {
                if matches!(info.kind, VariableKind::Value) {
                    info.schema_id
                } else {
                    None
                }
            });
            self.last_expr_type_info = binding_type;
            // Track numeric type for typed opcode emission
            self.last_expr_numeric_type = self
                .type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| Self::storage_hint_to_numeric_type(info.storage_hint));
        } else if let Some(func_idx) = self.find_function(name) {
            let resolved_name = self.program.functions[func_idx].name.clone();

            // Check if removed by comptime annotation handler.
            if self.removed_functions.contains(&resolved_name)
                || self.removed_functions.contains(name)
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "function '{}' was removed by a comptime annotation handler and cannot be referenced",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }

            let is_comptime_fn = self
                .function_defs
                .get(&resolved_name)
                .or_else(|| self.function_defs.get(name))
                .map(|def| def.is_comptime)
                .unwrap_or(false);
            if is_comptime_fn && !self.comptime_mode {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "'{}' is declared as `comptime fn` and can only be referenced from comptime contexts",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
            let const_idx = self
                .program
                .add_constant(Constant::Function(func_idx as u16));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(const_idx)),
            ));
            // Functions don't produce TypedObjects or numeric values
            self.last_expr_schema = None;
            self.last_expr_numeric_type = None;
            self.last_expr_type_info = None;
        } else {
            // Collect available names for "Did you mean?" suggestion
            let available = self.collect_available_names();
            let mut message = self.undefined_variable_message(name);
            if let Some(suggestion) = suggest_variable(name, &available) {
                message.push_str(&format!(". {}", suggestion));
            }
            return Err(ShapeError::RuntimeError {
                message,
                location: Some(self.span_to_source_location(span)),
            });
        }
        Ok(())
    }

    /// Collect all available variable and function names for suggestions
    fn collect_available_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        // Local variables from all scopes
        for scope in &self.locals {
            for name in scope.keys() {
                names.push(name.clone());
            }
        }
        // ModuleBinding variables
        for name in self.module_bindings.keys() {
            names.push(name.clone());
        }
        // Function names
        for func in &self.program.functions {
            names.push(func.name.clone());
        }
        names
    }
}
