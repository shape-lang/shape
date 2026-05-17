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
        // Mutable closure captures: dispatch by CaptureKind.
        //   * `CaptureKind::Shared`        → A.1B `LoadSharedCapture`.
        //   * `CaptureKind::OwnedMutable`  → A.1B `LoadOwnedMutableCapture`.
        //   * legacy SharedCell fallback   → `LoadClosure` (module-
        //     binding `var` captures that A.1C.1's outer-scope opcodes
        //     don't yet cover; retired in A.1C.3).
        if let Some(&upvalue_idx) = self.mutable_closure_captures.get(name) {
            // Track A.1C.2: Shared (var) captures route through the A.1B
            // LoadSharedCapture opcode, which takes the parking_lot mutex
            // on the `Arc<SharedCell>` pointer stored in the capture slot
            // and pushes the inner ValueWord bits.
            if let Some(&shared_idx) = self.shared_closure_captures.get(name) {
                debug_assert_eq!(upvalue_idx, shared_idx);
                // A2-refined / task #17: dispatch to Wave D.2's typed
                // `LoadSharedCapture<Kind>` opcodes (codes 0x156-0x160)
                // by looking up the cell's interior `FieldKind` from
                // `shared_capture_inner_kinds` (populated alongside
                // `shared_closure_captures` at closure-construction
                // time). Each typed opcode acquires the cell's mutex,
                // reads the matching native payload via
                // `read_shared_<kind>`, and pushes raw native bytes onto
                // the kinded VM stack via `push_kinded(bits, kind)`.
                // Falls back to the legacy
                // `LoadSharedCapture` (0x134) for unresolved capture
                // types — Wave G removes the legacy opcode after every
                // resolved emit path is type-aware.
                let opcode = match self.shared_capture_inner_kinds.get(name).copied() {
                    Some(kind) => crate::compiler::helpers::shared_typed_load_opcode(kind),
                    None => OpCode::LoadSharedCapture,
                };
                self.emit(Instruction::new(opcode, Some(Operand::Local(shared_idx))));
                self.last_expr_schema = None;
                self.last_expr_type_info = None;
                self.last_expr_numeric_type = None;
                return Ok(());
            }
            // Track A.1C.2b + Wave E: OwnedMutable (let mut) captures
            // route through Wave D.1's per-FieldKind typed opcodes
            // (codes 0x140-0x14A). The interior `FieldKind` was recorded
            // at closure-construction time in
            // `owned_mutable_capture_inner_kinds` from
            // `concrete_type_for_expr → ConcreteType::to_field_kind`.
            // Each typed opcode reads the matching native cell
            // (`*mut i64` / `*mut f64` / `*mut bool` / `*mut u64` for
            // Ptr) and pushes raw native bytes onto the kinded VM stack
            // via `push_kinded(bits, kind)` (sub-i64 ints sign- or
            // zero-extended into the i64 path). Dynamic / unresolved
            // capture types fall
            // back to the legacy `LoadOwnedMutableCapture` (0x132),
            // which handles the runtime dispatch on
            // `layout.capture_inner_kind(idx)` and re-encodes to a
            // ValueWord. Wave G removes the legacy opcode after every
            // resolved capture path is type-aware. The Shared (`var`)
            // capture path above stays on the legacy
            // `LoadSharedCapture` (0x134) — atomic flip is follow-up
            // #17.
            if let Some(&owned_idx) = self.owned_mutable_closure_captures.get(name) {
                debug_assert_eq!(upvalue_idx, owned_idx);
                let opcode = match self
                    .owned_mutable_capture_inner_kinds
                    .get(name)
                    .copied()
                {
                    Some(kind) => crate::compiler::helpers::owned_mutable_typed_load_opcode(kind),
                    None => OpCode::LoadOwnedMutableCapture,
                };
                self.emit(Instruction::new(opcode, Some(Operand::Local(owned_idx))));
                self.last_expr_schema = None;
                self.last_expr_type_info = None;
                self.last_expr_numeric_type = None;
                return Ok(());
            }
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
            // Session 1 — Rust-move for `let mut` captures. A `let mut`
            // local that has been captured by a closure (and therefore
            // routed through `CaptureKind::OwnedMutable` / moved by
            // value into the closure's `Box<ValueWord>`) cannot be read
            // from the outer scope afterwards: the outer slot holds a
            // stale snapshot. Reject at compile time.
            if let Some(&move_span) = self.captured_let_mut_moved.get(name) {
                return Err(Self::let_mut_use_after_move_error(
                    name, span, move_span, self, /*is_assign=*/ false,
                ));
            }
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
                //   UniqueHeap→ legacy cell + SharedCell, read via LoadClosure
                //   SharedCow → legacy cell + SharedCell, read via LoadClosure
                //   Reference → DerefLoad / DerefStore (handled above)
                //
                // Consult the MIR storage plan first (authoritative when available),
                // then fall back to type-tracker semantics for non-function contexts.
                let storage_class = self.mir_storage_class_for_slot(local_idx).or_else(|| {
                    self.type_tracker
                        .get_local_binding_semantics(local_idx)
                        .map(|s| s.storage_class)
                });

                if self.shared_locals.contains(name) {
                    // Track A.1C.2: the slot has been promoted to
                    // `Arc<SharedCell>` via `AllocSharedLocal`. Every
                    // subsequent outer-scope read must go through
                    // `LoadSharedLocal`, which takes the parking_lot
                    // mutex, reads the inner ValueWord bits, and pushes
                    // them onto the stack. Plain `LoadLocal` would push
                    // the raw `*const SharedCell` pointer bits, which
                    // subsequent arithmetic / dispatch would treat as
                    // an opaque word.
                    self.emit(Instruction::new(
                        OpCode::LoadSharedLocal,
                        Some(Operand::Local(local_idx)),
                    ));
                } else if self.boxed_locals.contains(name)
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
                                // Post-§2.7.5.1: `info.storage_hint` is
                                // `Option<StorageHint>`; the `Some(...)` arm
                                // gates on a proven primitive kind, `None`
                                // means "kind not yet proven" so no upgrade.
                                matches!(
                                    info.storage_hint,
                                    Some(
                                        StorageHint::Int64
                                            | StorageHint::Float64
                                            | StorageHint::Bool
                                    )
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
            // Track numeric type for typed opcode emission. Post-§2.7.5.1:
            // `info.storage_hint` is `Option<StorageHint>`, so we
            // `.and_then` through both layers — `None` propagates "kind not
            // yet proven" so no numeric type is recorded.
            self.last_expr_numeric_type = self
                .type_tracker
                .get_local_type(local_idx)
                .and_then(|info| info.storage_hint)
                .and_then(Self::storage_hint_to_numeric_type);
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
            } else if self.shared_module_bindings.contains(&scoped_name) {
                // Track A.1C.3: slot was promoted to
                // `Arc<SharedCell>` by a prior closure capture; read
                // through the mutex via `LoadSharedModuleBinding`.
                // Plain `LoadModuleBinding` would push the raw Arc
                // pointer bits, corrupting the downstream consumer.
                self.emit(Instruction::new(
                    OpCode::LoadSharedModuleBinding,
                    Some(Operand::ModuleBinding(binding_idx)),
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
            // Track numeric type for typed opcode emission. Post-§2.7.5.1:
            // `info.storage_hint` is `Option<StorageHint>`, so we
            // `.and_then` through both layers — `None` propagates "kind not
            // yet proven" so no numeric type is recorded.
            self.last_expr_numeric_type = self
                .type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| info.storage_hint)
                .and_then(Self::storage_hint_to_numeric_type);
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

    /// Session 1 — build the compile-time diagnostic emitted when a
    /// `let mut` binding that was moved into a closure is read (or
    /// written) in the outer scope. Mirrors Rust's E0382 /
    /// "borrow of moved value" error class: under the Rust-move
    /// semantics the user directive chose for `let mut`, the outer
    /// binding is consumed at the capture site, so subsequent uses
    /// are compile errors.
    fn let_mut_use_after_move_error(
        name: &str,
        use_span: Span,
        move_span: Span,
        compiler: &Self,
        is_assign: bool,
    ) -> ShapeError {
        let action = if is_assign { "assigned to" } else { "read" };
        ShapeError::SemanticError {
            message: format!(
                "[B0005] `let mut` binding '{name}' was moved into a closure here and cannot be \
                 {action} in the outer scope afterwards (Rust-move semantics). Use `var {name}` \
                 if the binding needs to be observed or mutated in the outer scope after capture, \
                 or observe mutations via the closure's return value."
            ),
            location: {
                // Prefer the use span for the primary location; the
                // move span flows through the message for context.
                let _ = move_span;
                Some(compiler.span_to_source_location(use_span))
            },
        }
    }
}
