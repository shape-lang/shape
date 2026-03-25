//! Helper methods for bytecode compilation

use super::BorrowMode;
use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::type_tracking::{NumericType, StorageHint, TypeTracker, VariableTypeInfo};
use shape_ast::ast::{Spanned, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};
use std::collections::{BTreeSet, HashMap};

use super::{
    BuiltinNameResolution, BytecodeCompiler, DropKind, ParamPassMode, ResolutionScope,
};

/// Extract the core error message from a ShapeError, stripping redundant
/// "Type error:", "Runtime error:", "Compile error:", etc. prefixes that
/// thiserror's Display impl adds.  This prevents nested comptime errors
/// from accumulating multiple prefixes like
/// "Runtime error: Comptime block evaluation failed: Runtime error: …".
pub(crate) fn strip_error_prefix(e: &ShapeError) -> String {
    let msg = e.to_string();
    // Known prefixes added by thiserror Display
    const PREFIXES: &[&str] = &[
        "Runtime error: ",
        "Type error: ",
        "Semantic error: ",
        "Parse error: ",
        "VM error: ",
        "Lexical error: ",
    ];
    let mut s = msg.as_str();
    // Strip at most 3 layers of prefix to handle deep nesting
    for _ in 0..3 {
        let mut stripped = false;
        for prefix in PREFIXES {
            if let Some(rest) = s.strip_prefix(prefix) {
                s = rest;
                stripped = true;
                break;
            }
        }
        // Also strip the comptime wrapping messages themselves
        const COMPTIME_PREFIXES: &[&str] = &[
            "Comptime block evaluation failed: ",
            "Comptime handler execution failed: ",
            "Comptime block directive processing failed: ",
        ];
        for prefix in COMPTIME_PREFIXES {
            if let Some(rest) = s.strip_prefix(prefix) {
                s = rest;
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    s.to_string()
}



impl BytecodeCompiler {
    fn scalar_type_name_from_numeric(numeric_type: NumericType) -> &'static str {
        match numeric_type {
            NumericType::Int | NumericType::IntWidth(_) => "int",
            NumericType::Number => "number",
            NumericType::Decimal => "decimal",
        }
    }

    fn array_type_name_from_numeric(numeric_type: NumericType) -> &'static str {
        match numeric_type {
            NumericType::Int | NumericType::IntWidth(_) => "Vec<int>",
            NumericType::Number => "Vec<number>",
            NumericType::Decimal => "Vec<decimal>",
        }
    }

    fn is_array_type_name(type_name: Option<&str>) -> bool {
        matches!(type_name, Some(name) if name.starts_with("Vec<") && name.ends_with('>'))
    }

    /// Convert a source annotation to a tracked type name when we have a
    /// canonical runtime representation for it.
    pub(super) fn tracked_type_name_from_annotation(type_ann: &TypeAnnotation) -> Option<String> {
        match type_ann {
            TypeAnnotation::Basic(name) => Some(name.clone()),
            TypeAnnotation::Reference(name) => Some(name.to_string()),
            TypeAnnotation::Array(inner) => Some(format!("Vec<{}>", inner.to_type_string())),
            // Keep the canonical Vec<T> naming even if a Generic slips through.
            TypeAnnotation::Generic { name, args } if name == "Vec" && args.len() == 1 => {
                Some(format!("Vec<{}>", args[0].to_type_string()))
            }
            TypeAnnotation::Generic { name, args } if name == "Mat" && args.len() == 1 => {
                Some(format!("Mat<{}>", args[0].to_type_string()))
            }
            // Track Option/Result wrapper types so conversion lifting can
            // detect them (even though generic args are lost in the tracker).
            TypeAnnotation::Generic { name, .. }
                if name == "Option" || name == "Result" =>
            {
                Some(name.to_lowercase())
            }
            _ => None,
        }
    }

    /// Resolve a type name through the module scope stack and imports.
    ///
    /// If the name is already directly known (in struct_types, type_aliases, etc.),
    /// returns it as-is. Otherwise, tries prefixing with each module scope from
    /// innermost to outermost, then checks imported names to find a match.
    pub(super) fn resolve_type_name(&self, name: &str) -> String {
        // Already qualified or directly found
        if name.contains("::") || self.is_type_known_direct(name) {
            return name.to_string();
        }
        // Try module scope prefixes (innermost to outermost)
        for scope in self.module_scope_stack.iter().rev() {
            let qualified = format!("{}::{}", scope, name);
            if self.is_type_known_direct(&qualified) {
                return qualified;
            }
        }
        // Check imported names (from `from ... use { Name }` imports)
        if let Some(imported) = self.imported_names.get(name) {
            // When module_path is set (graph-compiled dependency), prefer
            // module-qualified name. This prevents accidental binding to an
            // unrelated local/bare type of the same name.
            if !imported.module_path.is_empty() {
                let qualified = format!("{}::{}", imported.module_path, imported.original_name);
                if self.is_type_known_direct(&qualified) {
                    return qualified;
                }
            }
            // Fall back to bare original name (legacy imports without module_path)
            if self.is_type_known_direct(&imported.original_name) {
                return imported.original_name.clone();
            }
        }
        // Try namespace module prefixes (from `use module` imports)
        for ns in &self.module_namespace_bindings {
            let qualified = format!("{}::{}", ns, name);
            if self.is_type_known_direct(&qualified) {
                return qualified;
            }
            // Try canonical path for graph-compiled modules
            if let Some(canonical) = self.graph_namespace_map.get(ns) {
                let cq = format!("{}::{}", canonical, name);
                if self.is_type_known_direct(&cq) {
                    return cq;
                }
            }
        }
        // Return as-is (may be a forward reference or builtin)
        name.to_string()
    }

    /// Direct type lookup without scope resolution
    fn is_type_known_direct(&self, name: &str) -> bool {
        self.struct_types.contains_key(name)
            || self.type_aliases.contains_key(name)
            || self
                .type_inference
                .env
                .lookup_type_alias(name)
                .is_some()
            || self.type_inference.env.get_enum(name).is_some()
            || self
                .type_inference
                .env
                .lookup_interface(name)
                .is_some()
            || self.type_inference.env.lookup_trait(name).is_some()
            || self.type_tracker.schema_registry().get(name).is_some()
    }

    /// Resolve a trait name to its canonical form for definition lookup.
    ///
    /// Returns `(canonical_name, basename)` where `canonical_name` is used for
    /// `trait_defs` lookup and `basename` is used for dispatch registration
    /// (runtime dispatch keys are always bare basenames).
    pub(super) fn resolve_trait_name(&self, name: &str) -> (String, String) {
        let basename = name.rsplit("::").next().unwrap_or(name).to_string();
        // Check trait_defs in priority order
        if self.trait_defs.contains_key(name) {
            return (name.to_string(), basename);
        }
        for scope in self.module_scope_stack.iter().rev() {
            let q = format!("{}::{}", scope, name);
            if self.trait_defs.contains_key(&q) {
                return (q, basename);
            }
        }
        if let Some(imported) = self.imported_names.get(name) {
            if !imported.module_path.is_empty() {
                let q = format!("{}::{}", imported.module_path, imported.original_name);
                if self.trait_defs.contains_key(&q) {
                    return (q, basename);
                }
            }
        }
        for ns in &self.module_namespace_bindings {
            let q = format!("{}::{}", ns, name);
            if self.trait_defs.contains_key(&q) {
                return (q, basename);
            }
            if let Some(canonical) = self.graph_namespace_map.get(ns) {
                let cq = format!("{}::{}", canonical, name);
                if self.trait_defs.contains_key(&cq) {
                    return (cq, basename);
                }
            }
        }
        // Fall back to type_inference.env for built-in traits (Into, From, etc.)
        // registered bare in mod.rs but not in trait_defs.
        if self.type_inference.env.lookup_trait(name).is_some() {
            return (name.to_string(), basename);
        }
        if self.type_inference.env.lookup_trait(&basename).is_some() {
            return (basename.clone(), basename);
        }
        (name.to_string(), basename)
    }

    /// Mark a local/module binding slot as an array with numeric element type.
    ///
    /// Used by `x = x.push(value)` in-place mutation lowering so subsequent
    /// indexed reads can recover numeric hints.
    pub(super) fn mark_slot_as_numeric_array(
        &mut self,
        slot: u16,
        is_local: bool,
        numeric_type: NumericType,
    ) {
        let info =
            VariableTypeInfo::named(Self::array_type_name_from_numeric(numeric_type).to_string());
        if is_local {
            self.type_tracker.set_local_type(slot, info);
        } else {
            self.type_tracker.set_binding_type(slot, info);
        }
    }

    /// Mark a local/module binding slot as a scalar numeric type.
    pub(super) fn mark_slot_as_numeric_scalar(
        &mut self,
        slot: u16,
        is_local: bool,
        numeric_type: NumericType,
    ) {
        let info =
            VariableTypeInfo::named(Self::scalar_type_name_from_numeric(numeric_type).to_string());
        if is_local {
            self.type_tracker.set_local_type(slot, info);
        } else {
            self.type_tracker.set_binding_type(slot, info);
        }
    }

    /// Seed numeric hints from expression usage in arithmetic contexts.
    ///
    /// - `x` in numeric arithmetic becomes scalar numeric (`int`/`number`/`decimal`).
    /// - `arr[i]` implies `arr` is `Vec<numeric>`.
    pub(super) fn seed_numeric_hint_from_expr(
        &mut self,
        expr: &shape_ast::ast::Expr,
        numeric_type: NumericType,
    ) {
        match expr {
            shape_ast::ast::Expr::Identifier(name, _) => {
                if let Some(local_idx) = self.resolve_local(name) {
                    self.mark_slot_as_numeric_scalar(local_idx, true, numeric_type);
                    return;
                }
                let scoped_name = self
                    .resolve_scoped_module_binding_name(name)
                    .unwrap_or_else(|| name.to_string());
                if let Some(binding_idx) = self.module_bindings.get(&scoped_name).copied() {
                    self.mark_slot_as_numeric_scalar(binding_idx, false, numeric_type);
                }
            }
            shape_ast::ast::Expr::IndexAccess {
                object,
                end_index: None,
                ..
            } => {
                if let shape_ast::ast::Expr::Identifier(name, _) = object.as_ref() {
                    if let Some(local_idx) = self.resolve_local(name) {
                        self.mark_slot_as_numeric_array(local_idx, true, numeric_type);
                        return;
                    }
                    let scoped_name = self
                        .resolve_scoped_module_binding_name(name)
                        .unwrap_or_else(|| name.to_string());
                    if let Some(binding_idx) = self.module_bindings.get(&scoped_name).copied() {
                        self.mark_slot_as_numeric_array(binding_idx, false, numeric_type);
                    }
                }
            }
            _ => {}
        }
    }

    fn recover_or_bail_with_null_placeholder(&mut self, err: ShapeError) -> Result<()> {
        if self.should_recover_compile_diagnostics() {
            self.errors.push(err);
            self.emit(Instruction::simple(OpCode::PushNull));
            Ok(())
        } else {
            Err(err)
        }
    }

    pub(super) fn compile_expr_as_value_or_placeholder(
        &mut self,
        expr: &shape_ast::ast::Expr,
    ) -> Result<()> {
        match self.compile_expr(expr) {
            Ok(()) => Ok(()),
            Err(err) => self.recover_or_bail_with_null_placeholder(err),
        }
    }

    /// Emit an instruction and return its index
    /// Also records the current source line and file in debug info
    pub(super) fn emit(&mut self, instruction: Instruction) -> usize {
        let idx = self.program.emit(instruction);
        // Record line number and file for this instruction
        if self.current_line > 0 {
            self.program.debug_info.line_numbers.push((
                idx,
                self.current_file_id,
                self.current_line,
            ));
        }
        idx
    }

    /// Emit a boolean constant
    pub(super) fn emit_bool(&mut self, value: bool) {
        let const_idx = self.program.add_constant(Constant::Bool(value));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(const_idx)),
        ));
    }

    /// Emit a unit constant
    pub(super) fn emit_unit(&mut self) {
        let const_idx = self.program.add_constant(Constant::Unit);
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(const_idx)),
        ));
    }

    /// Emit a jump instruction with placeholder offset.
    ///
    /// When `opcode` is `JumpIfFalse` and the immediately preceding instruction
    /// is a typed or trusted comparison (produces a known bool), upgrades to
    /// `JumpIfFalseTrusted` which skips `is_truthy()` dispatch.
    pub(super) fn emit_jump(&mut self, mut opcode: OpCode, dummy: i32) -> usize {
        if opcode == OpCode::JumpIfFalse && self.last_instruction_produces_bool() {
            opcode = OpCode::JumpIfFalseTrusted;
        }
        self.emit(Instruction::new(opcode, Some(Operand::Offset(dummy))))
    }

    /// Returns true if the last emitted instruction always produces a boolean result.
    fn last_instruction_produces_bool(&self) -> bool {
        self.program
            .instructions
            .last()
            .map(|instr| {
                matches!(
                    instr.opcode,
                    OpCode::GtInt
                        | OpCode::GtNumber
                        | OpCode::GtDecimal
                        | OpCode::LtInt
                        | OpCode::LtNumber
                        | OpCode::LtDecimal
                        | OpCode::GteInt
                        | OpCode::GteNumber
                        | OpCode::GteDecimal
                        | OpCode::LteInt
                        | OpCode::LteNumber
                        | OpCode::LteDecimal
                        | OpCode::EqInt
                        | OpCode::EqNumber
                        | OpCode::NeqInt
                        | OpCode::NeqNumber
                        | OpCode::Gt
                        | OpCode::Lt
                        | OpCode::Gte
                        | OpCode::Lte
                        | OpCode::Eq
                        | OpCode::Neq
                        | OpCode::Not
                )
            })
            .unwrap_or(false)
    }

    /// Patch a jump instruction with the correct offset
    pub(super) fn patch_jump(&mut self, jump_idx: usize) {
        let offset = self.program.current_offset() as i32 - jump_idx as i32 - 1;
        self.program.instructions[jump_idx] = Instruction::new(
            self.program.instructions[jump_idx].opcode,
            Some(Operand::Offset(offset)),
        );
    }

    /// Compile function call arguments, enabling `&` reference expressions.
    ///
    /// Each call's arguments get their own borrow region so that borrows from
    /// `&` references are released after the call returns. This matches Rust's
    /// semantics: temporary borrows from function arguments don't persist beyond
    /// the call. Sequential calls like `inc(&a); inc(&a)` are correctly allowed.
    pub(super) fn compile_call_args(
        &mut self,
        args: &[shape_ast::ast::Expr],
        expected_param_modes: Option<&[ParamPassMode]>,
    ) -> Result<Vec<(u16, u16)>> {
        self.call_arg_module_binding_ref_writebacks.push(Vec::new());

        let mut first_error: Option<ShapeError> = None;
        for (idx, arg) in args.iter().enumerate() {
            let pass_mode = expected_param_modes
                .and_then(|modes| modes.get(idx).copied())
                .unwrap_or(ParamPassMode::ByValue);

            let arg_result = match pass_mode {
                ParamPassMode::ByRefExclusive | ParamPassMode::ByRefShared => {
                    let borrow_mode = if pass_mode.is_exclusive() {
                        BorrowMode::Exclusive
                    } else {
                        BorrowMode::Shared
                    };
                    if let shape_ast::ast::Expr::Reference { expr, span, .. } = arg {
                        self.compile_reference_expr(expr, *span, borrow_mode)
                            .map(|_| ())
                    } else {
                        self.compile_implicit_reference_arg(arg, borrow_mode)
                    }
                }
                ParamPassMode::ByValue => {
                    if let shape_ast::ast::Expr::Reference { span, .. } = arg {
                        let message = if expected_param_modes.is_some() {
                            "[B0004] unexpected `&` argument: target parameter is not a reference parameter".to_string()
                        } else {
                            "[B0004] cannot pass `&` to a callable value without a declared reference contract; \
                             call a named function with known parameter modes or add an explicit callable type"
                                .to_string()
                        };
                        Err(ShapeError::SemanticError {
                            message,
                            location: Some(self.span_to_source_location(*span)),
                        })
                    } else {
                        self.plan_flexible_binding_escape_from_expr(arg);
                        self.compile_expr(arg)
                    }
                }
            };

            if let Err(err) = arg_result {
                if self.should_recover_compile_diagnostics() {
                    self.errors.push(err);
                    // Keep stack arity consistent for downstream call codegen.
                    self.emit(Instruction::simple(OpCode::PushNull));
                    continue;
                }
                first_error = Some(err);
                break;
            }
        }

        let writebacks = self
            .call_arg_module_binding_ref_writebacks
            .pop()
            .unwrap_or_default();
        if let Some(err) = first_error {
            Err(err)
        } else {
            Ok(writebacks)
        }
    }

    pub(super) fn compile_implicit_reference_arg(
        &mut self,
        arg: &shape_ast::ast::Expr,
        mode: BorrowMode,
    ) -> Result<()> {
        use shape_ast::ast::Expr;
        match arg {
            Expr::Identifier(name, span) => self
                .compile_reference_identifier(name, *span, mode)
                .map(|_| ()),
            Expr::PropertyAccess {
                object,
                property,
                optional: false,
                span,
            } => self
                .compile_reference_property_access(object, property, *span, mode)
                .map(|_| ()),
            Expr::IndexAccess {
                object,
                index,
                end_index: None,
                span,
            } => self
                .compile_reference_index_access(object, index, *span, mode)
                .map(|_| ()),
            _ => {
                self.compile_expr_preserving_refs(arg)?;
                if let Some(returned_mode) = self.last_expr_reference_mode() {
                    if mode == BorrowMode::Exclusive && returned_mode != BorrowMode::Exclusive {
                        return Err(ShapeError::SemanticError {
                            message:
                                "cannot pass a shared reference result to an exclusive parameter"
                                    .to_string(),
                            location: Some(self.span_to_source_location(arg.span())),
                        });
                    }
                    return Ok(());
                }
                if mode == BorrowMode::Exclusive {
                    return Err(ShapeError::SemanticError {
                        message:
                            "[B0004] mutable reference arguments must be simple variables or existing exclusive references"
                                .to_string(),
                        location: Some(self.span_to_source_location(arg.span())),
                    });
                }
                let temp = self.declare_temp_local("__arg_ref_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(temp)),
                ));
                // MIR analysis is the sole authority for borrow checking.
                self.emit(Instruction::new(
                    OpCode::MakeRef,
                    Some(Operand::Local(temp)),
                ));
                Ok(())
            }
        }
    }

    pub(super) fn compile_reference_identifier(
        &mut self,
        name: &str,
        span: shape_ast::ast::Span,
        mode: BorrowMode,
    ) -> Result<u32> {
        if let Some(local_idx) = self.resolve_local(name) {
            // Reject exclusive borrows of const variables
            if mode == BorrowMode::Exclusive && self.const_locals.contains(&local_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot pass const variable '{}' by exclusive reference",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
            if self.ref_locals.contains(&local_idx) {
                // Forward an existing reference parameter by value (TAG_REF).
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(local_idx)),
                ));
                return Ok(u32::MAX);
            }
            if self.reference_value_locals.contains(&local_idx) {
                if mode == BorrowMode::Exclusive
                    && !self.exclusive_reference_value_locals.contains(&local_idx)
                {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Cannot pass shared reference variable '{}' as an exclusive reference",
                            name
                        ),
                        location: Some(self.span_to_source_location(span)),
                    });
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(local_idx)),
                ));
                return Ok(u32::MAX);
            }
            // MIR analysis is the sole authority for borrow checking.
            self.emit(Instruction::new(
                OpCode::MakeRef,
                Some(Operand::Local(local_idx)),
            ));
            Ok(u32::MAX)
        } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
            let Some(&binding_idx) = self.module_bindings.get(&scoped_name) else {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "[B0004] reference argument must be a local or module_binding variable, got '{}'",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            };
            // Reject exclusive borrows of const module bindings
            if mode == BorrowMode::Exclusive && self.const_module_bindings.contains(&binding_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot pass const variable '{}' by exclusive reference",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
            if self.reference_value_module_bindings.contains(&binding_idx) {
                if mode == BorrowMode::Exclusive
                    && !self
                        .exclusive_reference_value_module_bindings
                        .contains(&binding_idx)
                {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Cannot pass shared reference variable '{}' as an exclusive reference",
                            name
                        ),
                        location: Some(self.span_to_source_location(span)),
                    });
                }
                self.emit(Instruction::new(
                    OpCode::LoadModuleBinding,
                    Some(Operand::ModuleBinding(binding_idx)),
                ));
                return Ok(u32::MAX);
            }
            // MIR analysis is the sole authority for borrow checking.
            self.emit(Instruction::new(
                OpCode::MakeRef,
                Some(Operand::ModuleBinding(binding_idx)),
            ));
            Ok(u32::MAX)
        } else if let Some(func_idx) = self.find_function(name) {
            // Function name passed as reference argument: create a temporary local
            // with the function constant and make a reference to it.
            let temp = self.declare_temp_local("__fn_ref_")?;
            let const_idx = self
                .program
                .add_constant(Constant::Function(func_idx as u16));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(const_idx)),
            ));
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(temp)),
            ));
            // MIR analysis is the sole authority for borrow checking.
            self.emit(Instruction::new(
                OpCode::MakeRef,
                Some(Operand::Local(temp)),
            ));
            Ok(u32::MAX)
        } else {
            Err(ShapeError::SemanticError {
                message: format!(
                    "[B0004] reference argument must be a local or module_binding variable, got '{}'",
                    name
                ),
                location: Some(self.span_to_source_location(span)),
            })
        }
    }

    /// Push a new scope
    pub(super) fn push_scope(&mut self) {
        self.locals.push(HashMap::new());
        self.type_tracker.push_scope();
    }

    /// Pop a scope
    pub(super) fn pop_scope(&mut self) {
        self.locals.pop();
        self.type_tracker.pop_scope();
    }

    /// Declare a local variable
    pub(super) fn declare_local(&mut self, name: &str) -> Result<u16> {
        let idx = self.next_local;
        self.next_local += 1;

        if let Some(scope) = self.locals.last_mut() {
            scope.insert(name.to_string(), idx);
        }

        Ok(idx)
    }

    /// Resolve a local variable
    pub(super) fn resolve_local(&self, name: &str) -> Option<u16> {
        for scope in self.locals.iter().rev() {
            if let Some(&idx) = scope.get(name) {
                return Some(idx);
            }
        }
        None
    }

    /// Declare a temporary local variable
    pub(super) fn declare_temp_local(&mut self, prefix: &str) -> Result<u16> {
        let name = format!("{}{}", prefix, self.next_local);
        self.declare_local(&name)
    }

    /// Set type info for an existing local variable
    pub(super) fn set_local_type_info(&mut self, slot: u16, type_name: &str) {
        let info = if let Some(schema) = self.type_tracker.schema_registry().get(type_name) {
            VariableTypeInfo::known(schema.id, type_name.to_string())
        } else {
            VariableTypeInfo::named(type_name.to_string())
        };
        self.type_tracker.set_local_type(slot, info);
    }

    /// Set type info for a module_binding variable
    pub(super) fn set_module_binding_type_info(&mut self, slot: u16, type_name: &str) {
        let info = if let Some(schema) = self.type_tracker.schema_registry().get(type_name) {
            VariableTypeInfo::known(schema.id, type_name.to_string())
        } else {
            VariableTypeInfo::named(type_name.to_string())
        };
        self.type_tracker.set_binding_type(slot, info);
    }

    /// Capture local storage hints for a compiled function.
    ///
    /// Must be called before the function scope is popped so the type tracker still
    /// has local slot metadata. Also populates the function's `FrameDescriptor` so
    /// the verifier and executor can use per-slot type info for trusted opcodes.
    pub(super) fn capture_function_local_storage_hints(&mut self, func_idx: usize) {
        let Some(func) = self.program.functions.get(func_idx) else {
            return;
        };
        let hints: Vec<StorageHint> = (0..func.locals_count)
            .map(|slot| self.type_tracker.get_local_storage_hint(slot))
            .collect();

        // Populate FrameDescriptor on the function for trusted opcode verification.
        let has_any_known = hints.iter().any(|h| *h != StorageHint::Unknown);
        let instr_len = self.program.instructions.len();
        let code_end = if func.body_length > 0 {
            (func.entry_point + func.body_length).min(instr_len)
        } else {
            instr_len
        };
        let has_trusted = if func.entry_point <= code_end && code_end <= instr_len {
            self.program.instructions[func.entry_point..code_end]
                .iter()
                .any(|i| i.opcode.is_trusted())
        } else {
            false
        };
        if has_any_known || has_trusted {
            self.program.functions[func_idx].frame_descriptor = Some(
                crate::type_tracking::FrameDescriptor::from_slots(hints.clone()),
            );
        }

        if self.program.function_local_storage_hints.len() <= func_idx {
            self.program
                .function_local_storage_hints
                .resize(func_idx + 1, Vec::new());
        }
        self.program.function_local_storage_hints[func_idx] = hints;
    }

    /// Populate program-level storage hints for top-level locals and module bindings.
    pub(super) fn populate_program_storage_hints(&mut self) {
        let top_hints: Vec<StorageHint> = (0..self.next_local)
            .map(|slot| self.type_tracker.get_local_storage_hint(slot))
            .collect();
        self.program.top_level_local_storage_hints = top_hints.clone();

        // Build top-level FrameDescriptor so JIT can use per-slot type info
        let has_any_known = top_hints.iter().any(|h| *h != StorageHint::Unknown);
        let has_trusted = self
            .program
            .instructions
            .iter()
            .any(|i| i.opcode.is_trusted());
        if has_any_known || has_trusted {
            self.program.top_level_frame =
                Some(crate::type_tracking::FrameDescriptor::from_slots(top_hints));
        }

        let mut module_binding_hints = vec![StorageHint::Unknown; self.module_bindings.len()];
        for &idx in self.module_bindings.values() {
            if let Some(slot) = module_binding_hints.get_mut(idx as usize) {
                *slot = self.type_tracker.get_module_binding_storage_hint(idx);
            }
        }
        self.program.module_binding_storage_hints = module_binding_hints;

        if self.program.function_local_storage_hints.len() < self.program.functions.len() {
            self.program
                .function_local_storage_hints
                .resize(self.program.functions.len(), Vec::new());
        } else if self.program.function_local_storage_hints.len() > self.program.functions.len() {
            self.program
                .function_local_storage_hints
                .truncate(self.program.functions.len());
        }
    }

    /// Propagate the current expression's inferred type metadata to a target slot.
    ///
    /// Used by assignment sites to keep mutable locals/module_bindings typed when
    /// safe, and to clear stale hints when assigning unknown/dynamic values.
    pub(super) fn propagate_assignment_type_to_slot(
        &mut self,
        slot: u16,
        is_local: bool,
        allow_number_hint: bool,
    ) {
        if let Some(ref info) = self.last_expr_type_info {
            if info.is_indexed()
                || info.is_datatable()
                || info.schema_id.is_some()
                || Self::is_array_type_name(info.type_name.as_deref())
            {
                if is_local {
                    self.type_tracker.set_local_type(slot, info.clone());
                } else {
                    self.type_tracker.set_binding_type(slot, info.clone());
                }
                return;
            }
        }

        if let Some(schema_id) = self.last_expr_schema {
            let schema_name = self
                .type_tracker
                .schema_registry()
                .get_by_id(schema_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| format!("__anon_{}", schema_id));
            let info = VariableTypeInfo::known(schema_id, schema_name);
            if is_local {
                self.type_tracker.set_local_type(slot, info);
            } else {
                self.type_tracker.set_binding_type(slot, info);
            }
            return;
        }

        if let Some(numeric_type) = self.last_expr_numeric_type {
            let (type_name, hint) = match numeric_type {
                crate::type_tracking::NumericType::Int => ("int", StorageHint::Int64),
                crate::type_tracking::NumericType::IntWidth(w) => {
                    use shape_ast::IntWidth;
                    let hint = match w {
                        IntWidth::I8 => StorageHint::Int8,
                        IntWidth::U8 => StorageHint::UInt8,
                        IntWidth::I16 => StorageHint::Int16,
                        IntWidth::U16 => StorageHint::UInt16,
                        IntWidth::I32 => StorageHint::Int32,
                        IntWidth::U32 => StorageHint::UInt32,
                        IntWidth::U64 => StorageHint::UInt64,
                    };
                    (w.type_name(), hint)
                }
                crate::type_tracking::NumericType::Number => {
                    if !allow_number_hint {
                        if is_local {
                            self.type_tracker
                                .set_local_type(slot, VariableTypeInfo::unknown());
                        } else {
                            self.type_tracker
                                .set_binding_type(slot, VariableTypeInfo::unknown());
                        }
                        return;
                    }
                    ("number", StorageHint::Float64)
                }
                // Decimal typed opcodes are not JIT-compiled yet.
                crate::type_tracking::NumericType::Decimal => {
                    if is_local {
                        self.type_tracker
                            .set_local_type(slot, VariableTypeInfo::unknown());
                    } else {
                        self.type_tracker
                            .set_binding_type(slot, VariableTypeInfo::unknown());
                    }
                    return;
                }
            };
            let info = VariableTypeInfo::with_storage(type_name.to_string(), hint);
            if is_local {
                self.type_tracker.set_local_type(slot, info);
            } else {
                self.type_tracker.set_binding_type(slot, info);
            }
            return;
        }

        // Assignment to an unknown/dynamic expression invalidates prior hints.
        if is_local {
            self.type_tracker
                .set_local_type(slot, VariableTypeInfo::unknown());
        } else {
            self.type_tracker
                .set_binding_type(slot, VariableTypeInfo::unknown());
        }
    }

    /// Propagate current expression type metadata to an identifier target.
    ///
    /// Reference locals are skipped because assignment writes through to a pointee.
    pub(super) fn propagate_assignment_type_to_identifier(&mut self, name: &str) {
        if let Some(local_idx) = self.resolve_local(name) {
            if self.local_binding_is_reference_value(local_idx) {
                return;
            }
            self.propagate_assignment_type_to_slot(local_idx, true, true);
            return;
        }

        let scoped_name = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        let binding_idx = self.get_or_create_module_binding(&scoped_name);
        self.propagate_assignment_type_to_slot(binding_idx, false, true);
    }

    /// Get the type tracker (for external configuration)
    /// Resolve a local namespace name to its canonical module path.
    ///
    /// Checks `graph_namespace_map` first (populated by graph-driven compilation),
    /// then falls back to `module_scope_sources` (legacy AST inlining path).
    pub(crate) fn resolve_canonical_module_path(&self, local_name: &str) -> Option<String> {
        self.graph_namespace_map
            .get(local_name)
            .or_else(|| self.module_scope_sources.get(local_name))
            .cloned()
    }

    pub fn type_tracker(&self) -> &TypeTracker {
        &self.type_tracker
    }

    /// Get mutable type tracker (for registering types)
    pub fn type_tracker_mut(&mut self) -> &mut TypeTracker {
        &mut self.type_tracker
    }

    /// Resolve a column name to its index using the data schema.
    /// Returns an error if no schema is provided or the column doesn't exist.
    pub(super) fn resolve_column_index(&self, field: &str) -> Result<u32> {
        self.program
            .data_schema
            .as_ref()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "No data schema provided. Cannot resolve field '{}'. \
                     Hint: Use stdlib/finance to load market data with OHLCV schema.",
                    field
                ),
                location: None,
            })?
            .get_index(field)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Unknown column '{}' in data schema. Available columns: {:?}",
                    field,
                    self.program
                        .data_schema
                        .as_ref()
                        .map(|s| &s.column_names)
                        .unwrap_or(&vec![])
                ),
                location: None,
            })
    }

    /// Check if a field name is a known data column in the schema.
    pub(super) fn is_data_column(&self, field: &str) -> bool {
        self.program
            .data_schema
            .as_ref()
            .map(|s| s.get_index(field).is_some())
            .unwrap_or(false)
    }

    /// Collect all outer scope variables
    pub(super) fn collect_outer_scope_vars(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for scope in &self.locals {
            for name in scope.keys() {
                names.insert(name.clone());
            }
        }
        for name in self.module_bindings.keys() {
            names.insert(name.clone());
        }
        names.into_iter().collect()
    }

    /// Get or create a module_binding variable
    pub(super) fn get_or_create_module_binding(&mut self, name: &str) -> u16 {
        if let Some(&idx) = self.module_bindings.get(name) {
            idx
        } else {
            let idx = self.next_global;
            self.next_global += 1;
            self.module_bindings.insert(name.to_string(), idx);
            idx
        }
    }

    pub(super) fn resolve_scoped_module_binding_name(&self, name: &str) -> Option<String> {
        if crate::module_resolution::is_hidden_annotation_import_module_name(name) {
            return None;
        }
        if self.module_bindings.contains_key(name) {
            return Some(name.to_string());
        }
        for module_path in self.module_scope_stack.iter().rev() {
            let candidate = format!("{}::{}", module_path, name);
            if self.module_bindings.contains_key(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    pub(super) fn resolve_scoped_function_name(&self, name: &str) -> Option<String> {
        if self.program.functions.iter().any(|f| f.name == name) {
            return Some(name.to_string());
        }
        for module_path in self.module_scope_stack.iter().rev() {
            let candidate = format!("{}::{}", module_path, name);
            if self.program.functions.iter().any(|f| f.name == candidate) {
                return Some(candidate);
            }
        }
        None
    }

    /// Find a function by name
    pub(super) fn find_function(&self, name: &str) -> Option<usize> {
        // Check function aliases first (e.g., __original__ -> shadow function).
        if let Some(actual_name) = self.function_aliases.get(name) {
            if let Some(idx) = self
                .program
                .functions
                .iter()
                .position(|f| f.name == *actual_name)
            {
                return Some(idx);
            }
        }

        // Try direct/scoped resolution
        if let Some(resolved) = self.resolve_scoped_function_name(name) {
            if let Some(idx) = self
                .program
                .functions
                .iter()
                .position(|f| f.name == resolved)
            {
                return Some(idx);
            }
        }

        // If direct lookup failed, check imported_names for alias -> original name mapping.
        // When a function is imported with an alias (e.g., `use { foo as bar } from "module"`),
        // the function is registered under its original (possibly module-qualified) name,
        // but the user refers to it by the alias.
        if let Some(imported) = self.imported_names.get(name) {
            let original = &imported.original_name;
            // Try direct match on the original name
            if let Some(idx) = self
                .program
                .functions
                .iter()
                .position(|f| f.name == *original)
            {
                return Some(idx);
            }
            // Try scoped resolution on the original name
            if let Some(resolved) = self.resolve_scoped_function_name(original) {
                if let Some(idx) = self
                    .program
                    .functions
                    .iter()
                    .position(|f| f.name == resolved)
                {
                    return Some(idx);
                }
            }
            // Try module-qualified name: module_path::original_name
            // This is needed for graph-compiled dependencies where functions
            // are registered with their module-qualified names.
            if !imported.module_path.is_empty() {
                let qualified = format!("{}::{}", imported.module_path, original);
                if let Some(idx) = self
                    .program
                    .functions
                    .iter()
                    .position(|f| f.name == qualified)
                {
                    return Some(idx);
                }
            }
        }

        None
    }

    /// Resolve the receiver's type name for extend method dispatch.
    ///
    /// Determines the Shape type name from all available compiler state:
    /// - `last_expr_type_info.type_name` for TypedObjects (e.g., "Point", "Candle")
    /// - `last_expr_numeric_type` for numeric types → "Int", "Number", "Decimal"
    /// - Receiver expression analysis for arrays, strings, booleans
    ///
    /// Returns the base type name (e.g., "Vec" not "Vec<int>") suitable for
    /// extend method lookup as "Type.method".
    pub(super) fn resolve_receiver_extend_type(
        &self,
        receiver: &shape_ast::ast::Expr,
        receiver_type_info: &Option<crate::type_tracking::VariableTypeInfo>,
        _receiver_schema: Option<u32>,
    ) -> Option<String> {
        // 1. Numeric type from typed opcode tracking — checked first because
        //    the type tracker stores lowercase names ("int", "number") while
        //    extend blocks use capitalized TypeName ("Int", "Number", "Decimal").
        if let Some(numeric) = self.last_expr_numeric_type {
            return Some(
                match numeric {
                    crate::type_tracking::NumericType::Int
                    | crate::type_tracking::NumericType::IntWidth(_) => "Int",
                    crate::type_tracking::NumericType::Number => "Number",
                    crate::type_tracking::NumericType::Decimal => "Decimal",
                }
                .to_string(),
            );
        }

        // 2. TypedObject type name (user-defined types like Point, Candle)
        if let Some(info) = receiver_type_info {
            if let Some(type_name) = &info.type_name {
                // Strip generic params: "Vec<int>" → "Vec"
                let base = type_name.split('<').next().unwrap_or(type_name);
                return Some(base.to_string());
            }
        }

        // 3. Infer from receiver expression shape
        match receiver {
            shape_ast::ast::Expr::Literal(lit, _) => match lit {
                shape_ast::ast::Literal::String(_)
                | shape_ast::ast::Literal::FormattedString { .. }
                | shape_ast::ast::Literal::ContentString { .. } => Some("String".to_string()),
                shape_ast::ast::Literal::Bool(_) => Some("Bool".to_string()),
                _ => None,
            },
            shape_ast::ast::Expr::Array(..) => Some("Vec".to_string()),
            _ => None,
        }
    }

    /// Emit store instruction for an identifier
    pub(super) fn emit_store_identifier(&mut self, name: &str) -> Result<()> {
        // Mutable closure captures: emit StoreClosure to write to the shared upvalue
        if let Some(&upvalue_idx) = self.mutable_closure_captures.get(name) {
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
                        location: None,
                    });
                }
                self.emit(Instruction::new(
                    OpCode::DerefStore,
                    Some(Operand::Local(local_idx)),
                ));
            } else {
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(local_idx)),
                ));
                // Patch StoreLocal → StoreLocalTyped for width-typed locals
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
        } else {
            let scoped_name = self
                .resolve_scoped_module_binding_name(name)
                .unwrap_or_else(|| name.to_string());
            let binding_idx = self.get_or_create_module_binding(&scoped_name);
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
        }
        Ok(())
    }

    pub(super) fn classify_builtin_function(&self, name: &str) -> Option<BuiltinNameResolution> {
        let builtin = match name {
            // Option type constructor
            "Some" => BuiltinFunction::SomeCtor,
            "Ok" => BuiltinFunction::OkCtor,
            "Err" => BuiltinFunction::ErrCtor,
            "HashMap" => BuiltinFunction::HashMapCtor,
            "Set" => BuiltinFunction::SetCtor,
            "Deque" => BuiltinFunction::DequeCtor,
            "PriorityQueue" => BuiltinFunction::PriorityQueueCtor,
            "Mutex" => BuiltinFunction::MutexCtor,
            "Atomic" => BuiltinFunction::AtomicCtor,
            "Lazy" => BuiltinFunction::LazyCtor,
            "Channel" => BuiltinFunction::ChannelCtor,
            // Json navigation helpers
            "__json_object_get" => BuiltinFunction::JsonObjectGet,
            "__json_array_at" => BuiltinFunction::JsonArrayAt,
            "__json_object_keys" => BuiltinFunction::JsonObjectKeys,
            "__json_array_len" => BuiltinFunction::JsonArrayLen,
            "__json_object_len" => BuiltinFunction::JsonObjectLen,
            "__intrinsic_vec_abs" => BuiltinFunction::IntrinsicVecAbs,
            "__intrinsic_vec_sqrt" => BuiltinFunction::IntrinsicVecSqrt,
            "__intrinsic_vec_ln" => BuiltinFunction::IntrinsicVecLn,
            "__intrinsic_vec_exp" => BuiltinFunction::IntrinsicVecExp,
            "__intrinsic_vec_add" => BuiltinFunction::IntrinsicVecAdd,
            "__intrinsic_vec_sub" => BuiltinFunction::IntrinsicVecSub,
            "__intrinsic_vec_mul" => BuiltinFunction::IntrinsicVecMul,
            "__intrinsic_vec_div" => BuiltinFunction::IntrinsicVecDiv,
            "__intrinsic_vec_max" => BuiltinFunction::IntrinsicVecMax,
            "__intrinsic_vec_min" => BuiltinFunction::IntrinsicVecMin,
            "__intrinsic_vec_select" => BuiltinFunction::IntrinsicVecSelect,
            "__intrinsic_matmul_vec" => BuiltinFunction::IntrinsicMatMulVec,
            "__intrinsic_matmul_mat" => BuiltinFunction::IntrinsicMatMulMat,

            // Existing builtins
            "abs" => BuiltinFunction::Abs,
            "min" => BuiltinFunction::Min,
            "max" => BuiltinFunction::Max,
            "sqrt" => BuiltinFunction::Sqrt,
            "ln" => BuiltinFunction::Ln,
            "pow" => BuiltinFunction::Pow,
            "exp" => BuiltinFunction::Exp,
            "log" => BuiltinFunction::Log,
            "floor" => BuiltinFunction::Floor,
            "ceil" => BuiltinFunction::Ceil,
            "round" => BuiltinFunction::Round,
            "sin" => BuiltinFunction::Sin,
            "cos" => BuiltinFunction::Cos,
            "tan" => BuiltinFunction::Tan,
            "asin" => BuiltinFunction::Asin,
            "acos" => BuiltinFunction::Acos,
            "atan" => BuiltinFunction::Atan,
            "stddev" => BuiltinFunction::StdDev,
            "__intrinsic_map" => BuiltinFunction::Map,
            "__intrinsic_filter" => BuiltinFunction::Filter,
            "__intrinsic_reduce" => BuiltinFunction::Reduce,
            "print" => BuiltinFunction::Print,
            "format" => BuiltinFunction::Format,
            "len" | "count" => BuiltinFunction::Len,
            // "throw" removed: Shape uses Result types
            "__intrinsic_snapshot" | "snapshot" => BuiltinFunction::Snapshot,
            "exit" => BuiltinFunction::Exit,
            "range" => BuiltinFunction::Range,
            "is_number" | "isNumber" => BuiltinFunction::IsNumber,
            "is_string" | "isString" => BuiltinFunction::IsString,
            "is_bool" | "isBool" => BuiltinFunction::IsBool,
            "is_array" | "isArray" => BuiltinFunction::IsArray,
            "is_object" | "isObject" => BuiltinFunction::IsObject,
            "is_data_row" | "isDataRow" => BuiltinFunction::IsDataRow,
            "to_string" | "toString" => BuiltinFunction::ToString,
            "to_number" | "toNumber" => BuiltinFunction::ToNumber,
            "to_bool" | "toBool" => BuiltinFunction::ToBool,
            // __into_*/__try_into_* builtins removed — primitive conversions now use
            // typed ConvertTo*/TryConvertTo* opcodes emitted directly by the compiler.
            "__native_ptr_size" => BuiltinFunction::NativePtrSize,
            "__native_ptr_new_cell" => BuiltinFunction::NativePtrNewCell,
            "__native_ptr_free_cell" => BuiltinFunction::NativePtrFreeCell,
            "__native_ptr_read_ptr" => BuiltinFunction::NativePtrReadPtr,
            "__native_ptr_write_ptr" => BuiltinFunction::NativePtrWritePtr,
            "__native_table_from_arrow_c" => BuiltinFunction::NativeTableFromArrowC,
            "__native_table_from_arrow_c_typed" => BuiltinFunction::NativeTableFromArrowCTyped,
            "__native_table_bind_type" => BuiltinFunction::NativeTableBindType,
            "fold" => BuiltinFunction::ControlFold,

            // Math intrinsics
            "__intrinsic_minimize" => BuiltinFunction::IntrinsicMinimize,
            "__intrinsic_bspline2_3d_batch" => BuiltinFunction::IntrinsicBspline2_3dBatch,
            "__intrinsic_sum" => BuiltinFunction::IntrinsicSum,
            "__intrinsic_mean" => BuiltinFunction::IntrinsicMean,
            "__intrinsic_min" => BuiltinFunction::IntrinsicMin,
            "__intrinsic_max" => BuiltinFunction::IntrinsicMax,
            "__intrinsic_std" => BuiltinFunction::IntrinsicStd,
            "__intrinsic_variance" => BuiltinFunction::IntrinsicVariance,

            // Random intrinsics
            "__intrinsic_random" => BuiltinFunction::IntrinsicRandom,
            "__intrinsic_random_int" => BuiltinFunction::IntrinsicRandomInt,
            "__intrinsic_random_seed" => BuiltinFunction::IntrinsicRandomSeed,
            "__intrinsic_random_normal" => BuiltinFunction::IntrinsicRandomNormal,
            "__intrinsic_random_array" => BuiltinFunction::IntrinsicRandomArray,

            // Distribution intrinsics
            "__intrinsic_dist_uniform" => BuiltinFunction::IntrinsicDistUniform,
            "__intrinsic_dist_lognormal" => BuiltinFunction::IntrinsicDistLognormal,
            "__intrinsic_dist_exponential" => BuiltinFunction::IntrinsicDistExponential,
            "__intrinsic_dist_poisson" => BuiltinFunction::IntrinsicDistPoisson,
            "__intrinsic_dist_sample_n" => BuiltinFunction::IntrinsicDistSampleN,

            // Stochastic process intrinsics
            "__intrinsic_brownian_motion" => BuiltinFunction::IntrinsicBrownianMotion,
            "__intrinsic_gbm" => BuiltinFunction::IntrinsicGbm,
            "__intrinsic_ou_process" => BuiltinFunction::IntrinsicOuProcess,
            "__intrinsic_random_walk" => BuiltinFunction::IntrinsicRandomWalk,

            // Rolling intrinsics
            "__intrinsic_rolling_sum" => BuiltinFunction::IntrinsicRollingSum,
            "__intrinsic_rolling_mean" => BuiltinFunction::IntrinsicRollingMean,
            "__intrinsic_rolling_std" => BuiltinFunction::IntrinsicRollingStd,
            "__intrinsic_rolling_min" => BuiltinFunction::IntrinsicRollingMin,
            "__intrinsic_rolling_max" => BuiltinFunction::IntrinsicRollingMax,
            "__intrinsic_ema" => BuiltinFunction::IntrinsicEma,
            "__intrinsic_linear_recurrence" => BuiltinFunction::IntrinsicLinearRecurrence,

            // Series intrinsics
            "__intrinsic_shift" => BuiltinFunction::IntrinsicShift,
            "__intrinsic_diff" => BuiltinFunction::IntrinsicDiff,
            "__intrinsic_pct_change" => BuiltinFunction::IntrinsicPctChange,
            "__intrinsic_fillna" => BuiltinFunction::IntrinsicFillna,
            "__intrinsic_cumsum" => BuiltinFunction::IntrinsicCumsum,
            "__intrinsic_cumprod" => BuiltinFunction::IntrinsicCumprod,
            "__intrinsic_clip" => BuiltinFunction::IntrinsicClip,

            // Trigonometric intrinsics (map __intrinsic_ forms to existing builtins)
            "__intrinsic_sin" => BuiltinFunction::Sin,
            "__intrinsic_cos" => BuiltinFunction::Cos,
            "__intrinsic_tan" => BuiltinFunction::Tan,
            "__intrinsic_asin" => BuiltinFunction::Asin,
            "__intrinsic_acos" => BuiltinFunction::Acos,
            "__intrinsic_atan" => BuiltinFunction::Atan,
            "__intrinsic_atan2" => BuiltinFunction::IntrinsicAtan2,
            "__intrinsic_sinh" => BuiltinFunction::IntrinsicSinh,
            "__intrinsic_cosh" => BuiltinFunction::IntrinsicCosh,
            "__intrinsic_tanh" => BuiltinFunction::IntrinsicTanh,

            // Statistical intrinsics
            "__intrinsic_correlation" => BuiltinFunction::IntrinsicCorrelation,
            "__intrinsic_covariance" => BuiltinFunction::IntrinsicCovariance,
            "__intrinsic_percentile" => BuiltinFunction::IntrinsicPercentile,
            "__intrinsic_median" => BuiltinFunction::IntrinsicMedian,

            // Character code intrinsics
            "__intrinsic_char_code" => BuiltinFunction::IntrinsicCharCode,
            "__intrinsic_from_char_code" => BuiltinFunction::IntrinsicFromCharCode,

            // Series access
            "__intrinsic_series" => BuiltinFunction::IntrinsicSeries,

            // Reflection
            "reflect" => BuiltinFunction::Reflect,

            // Additional math builtins
            "sign" => BuiltinFunction::Sign,
            "gcd" => BuiltinFunction::Gcd,
            "lcm" => BuiltinFunction::Lcm,
            "hypot" => BuiltinFunction::Hypot,
            "clamp" => BuiltinFunction::Clamp,
            "isNaN" | "is_nan" => BuiltinFunction::IsNaN,
            "isFinite" | "is_finite" => BuiltinFunction::IsFinite,
            "mat" => BuiltinFunction::MatFromFlat,
            _ => return None,
        };

        let scope = match name {
            "Some" | "Ok" | "Err" => ResolutionScope::TypeAssociated,
            "print" => ResolutionScope::Prelude,
            _ if Self::is_internal_intrinsic_name(name) => ResolutionScope::InternalIntrinsic,
            _ => ResolutionScope::ModuleBinding,
        };

        Some(match scope {
            ResolutionScope::InternalIntrinsic => {
                BuiltinNameResolution::InternalOnly { builtin, scope }
            }
            _ => BuiltinNameResolution::Surface { builtin, scope },
        })
    }

    pub(super) fn is_internal_intrinsic_name(name: &str) -> bool {
        name.starts_with("__native_")
            || name.starts_with("__intrinsic_")
            || name.starts_with("__json_")
    }

    pub(super) const fn variable_scope_summary() -> &'static str {
        "Variable names resolve from local scope and module scope."
    }

    pub(super) const fn function_scope_summary() -> &'static str {
        "Function names resolve from module scope, explicit imports, type-associated scope, and the implicit prelude."
    }

    pub(super) fn undefined_variable_message(&self, name: &str) -> String {
        format!(
            "Undefined variable: {}. {}",
            name,
            Self::variable_scope_summary()
        )
    }

    pub(super) fn undefined_function_message(&self, name: &str) -> String {
        format!(
            "Undefined function: {}. {}",
            name,
            Self::function_scope_summary()
        )
    }

    pub(super) fn internal_intrinsic_error_message(
        &self,
        name: &str,
        resolution: BuiltinNameResolution,
    ) -> String {
        format!(
            "'{}' resolves to {} and is not available from ordinary user code. Internal intrinsics are reserved for std::* implementations and compiler-generated code.",
            name,
            resolution.scope().label()
        )
    }

    /// Check if a builtin function requires arg count
    pub(super) fn builtin_requires_arg_count(&self, builtin: BuiltinFunction) -> bool {
        matches!(
            builtin,
            BuiltinFunction::Abs
                | BuiltinFunction::Min
                | BuiltinFunction::Max
                | BuiltinFunction::Sqrt
                | BuiltinFunction::Ln
                | BuiltinFunction::Pow
                | BuiltinFunction::Exp
                | BuiltinFunction::Log
                | BuiltinFunction::Floor
                | BuiltinFunction::Ceil
                | BuiltinFunction::Round
                | BuiltinFunction::Sin
                | BuiltinFunction::Cos
                | BuiltinFunction::Tan
                | BuiltinFunction::Asin
                | BuiltinFunction::Acos
                | BuiltinFunction::Atan
                | BuiltinFunction::StdDev
                | BuiltinFunction::Range
                | BuiltinFunction::Slice
                | BuiltinFunction::Push
                | BuiltinFunction::Pop
                | BuiltinFunction::First
                | BuiltinFunction::Last
                | BuiltinFunction::Zip
                | BuiltinFunction::Map
                | BuiltinFunction::Filter
                | BuiltinFunction::Reduce
                | BuiltinFunction::ForEach
                | BuiltinFunction::Find
                | BuiltinFunction::FindIndex
                | BuiltinFunction::Some
                | BuiltinFunction::Every
                | BuiltinFunction::SomeCtor
                | BuiltinFunction::OkCtor
                | BuiltinFunction::ErrCtor
                | BuiltinFunction::HashMapCtor
                | BuiltinFunction::SetCtor
                | BuiltinFunction::DequeCtor
                | BuiltinFunction::PriorityQueueCtor
                | BuiltinFunction::MutexCtor
                | BuiltinFunction::AtomicCtor
                | BuiltinFunction::LazyCtor
                | BuiltinFunction::ChannelCtor
                | BuiltinFunction::Print
                | BuiltinFunction::Format
                | BuiltinFunction::Len
                // BuiltinFunction::Throw removed
                | BuiltinFunction::Snapshot
                | BuiltinFunction::ObjectRest
                | BuiltinFunction::IsNumber
                | BuiltinFunction::IsString
                | BuiltinFunction::IsBool
                | BuiltinFunction::IsArray
                | BuiltinFunction::IsObject
                | BuiltinFunction::IsDataRow
                | BuiltinFunction::ToString
                | BuiltinFunction::ToNumber
                | BuiltinFunction::ToBool
                | BuiltinFunction::NativePtrSize
                | BuiltinFunction::NativePtrNewCell
                | BuiltinFunction::NativePtrFreeCell
                | BuiltinFunction::NativePtrReadPtr
                | BuiltinFunction::NativePtrWritePtr
                | BuiltinFunction::NativeTableFromArrowC
                | BuiltinFunction::NativeTableFromArrowCTyped
                | BuiltinFunction::NativeTableBindType
                | BuiltinFunction::ControlFold
                | BuiltinFunction::IntrinsicMinimize
                | BuiltinFunction::IntrinsicBspline2_3dBatch
                | BuiltinFunction::IntrinsicSum
                | BuiltinFunction::IntrinsicMean
                | BuiltinFunction::IntrinsicMin
                | BuiltinFunction::IntrinsicMax
                | BuiltinFunction::IntrinsicStd
                | BuiltinFunction::IntrinsicVariance
                | BuiltinFunction::IntrinsicRandom
                | BuiltinFunction::IntrinsicRandomInt
                | BuiltinFunction::IntrinsicRandomSeed
                | BuiltinFunction::IntrinsicRandomNormal
                | BuiltinFunction::IntrinsicRandomArray
                | BuiltinFunction::IntrinsicDistUniform
                | BuiltinFunction::IntrinsicDistLognormal
                | BuiltinFunction::IntrinsicDistExponential
                | BuiltinFunction::IntrinsicDistPoisson
                | BuiltinFunction::IntrinsicDistSampleN
                | BuiltinFunction::IntrinsicBrownianMotion
                | BuiltinFunction::IntrinsicGbm
                | BuiltinFunction::IntrinsicOuProcess
                | BuiltinFunction::IntrinsicRandomWalk
                | BuiltinFunction::IntrinsicRollingSum
                | BuiltinFunction::IntrinsicRollingMean
                | BuiltinFunction::IntrinsicRollingStd
                | BuiltinFunction::IntrinsicRollingMin
                | BuiltinFunction::IntrinsicRollingMax
                | BuiltinFunction::IntrinsicEma
                | BuiltinFunction::IntrinsicLinearRecurrence
                | BuiltinFunction::IntrinsicShift
                | BuiltinFunction::IntrinsicDiff
                | BuiltinFunction::IntrinsicPctChange
                | BuiltinFunction::IntrinsicFillna
                | BuiltinFunction::IntrinsicCumsum
                | BuiltinFunction::IntrinsicCumprod
                | BuiltinFunction::IntrinsicClip
                | BuiltinFunction::IntrinsicCorrelation
                | BuiltinFunction::IntrinsicCovariance
                | BuiltinFunction::IntrinsicPercentile
                | BuiltinFunction::IntrinsicMedian
                | BuiltinFunction::IntrinsicAtan2
                | BuiltinFunction::IntrinsicSinh
                | BuiltinFunction::IntrinsicCosh
                | BuiltinFunction::IntrinsicTanh
                | BuiltinFunction::IntrinsicCharCode
                | BuiltinFunction::IntrinsicFromCharCode
                | BuiltinFunction::IntrinsicSeries
                | BuiltinFunction::IntrinsicVecAbs
                | BuiltinFunction::IntrinsicVecSqrt
                | BuiltinFunction::IntrinsicVecLn
                | BuiltinFunction::IntrinsicVecExp
                | BuiltinFunction::IntrinsicVecAdd
                | BuiltinFunction::IntrinsicVecSub
                | BuiltinFunction::IntrinsicVecMul
                | BuiltinFunction::IntrinsicVecDiv
                | BuiltinFunction::IntrinsicVecMax
                | BuiltinFunction::IntrinsicVecMin
                | BuiltinFunction::IntrinsicVecSelect
                | BuiltinFunction::IntrinsicMatMulVec
                | BuiltinFunction::IntrinsicMatMulMat
                | BuiltinFunction::Sign
                | BuiltinFunction::Gcd
                | BuiltinFunction::Lcm
                | BuiltinFunction::Hypot
                | BuiltinFunction::Clamp
                | BuiltinFunction::IsNaN
                | BuiltinFunction::IsFinite
                | BuiltinFunction::MatFromFlat
        )
    }

    /// Check if any compiled function exists whose name indicates a user-defined
    /// override of the given method name (via extend blocks or impl blocks).
    ///
    /// Looks for function names like `Type.method` or `Type::method`.
    pub(super) fn has_any_user_defined_method(&self, method: &str) -> bool {
        let dot_suffix = format!(".{}", method);
        let colon_suffix = format!("::{}", method);
        self.program
            .functions
            .iter()
            .any(|f| f.name.ends_with(&dot_suffix) || f.name.ends_with(&colon_suffix))
    }

    /// Check if a method name is a known built-in method on any VM type.
    /// Used by UFCS to determine if `receiver.method(args)` should be dispatched
    /// as a built-in method call or rewritten to `method(receiver, args)`.
    pub(super) fn is_known_builtin_method(method: &str) -> bool {
        // Array methods (from ARRAY_METHODS PHF map)
        matches!(method,
            "map" | "filter" | "reduce" | "forEach" | "find" | "findIndex"
            | "some" | "every" | "sort" | "groupBy" | "flatMap"
            | "len" | "length" | "first" | "last" | "reverse" | "slice"
            | "concat" | "take" | "drop" | "skip"
            | "indexOf" | "includes"
            | "join" | "flatten" | "unique" | "distinct" | "distinctBy"
            | "sum" | "avg" | "min" | "max" | "count"
            | "where" | "select" | "orderBy" | "thenBy" | "takeWhile"
            | "skipWhile" | "single" | "any" | "all"
            | "innerJoin" | "leftJoin" | "crossJoin"
            | "union" | "intersect" | "except"
        )
        // DataTable methods (from DATATABLE_METHODS PHF map)
        || matches!(method,
            "columns" | "column" | "head" | "tail" | "mean" | "std"
            | "describe" | "aggregate" | "group_by" | "index_by" | "indexBy"
            | "simulate" | "toMat" | "to_mat"
        )
        // Column methods (from COLUMN_METHODS PHF map)
        || matches!(method, "toArray")
        // IndexedTable methods (from INDEXED_TABLE_METHODS PHF map)
        || matches!(method, "resample" | "between")
        // Number methods handled inline in op_call_method
        || matches!(method,
            "toFixed" | "toInt" | "toNumber" | "to_number" | "floor" | "ceil" | "round"
            | "abs" | "sign" | "clamp"
        )
        // String methods handled inline
        || matches!(method,
            "toUpperCase" | "toLowerCase" | "trim" | "contains" | "startsWith"
            | "endsWith" | "split" | "replace" | "substring" | "charAt"
            | "padStart" | "padEnd" | "repeat" | "toString"
        )
        // Object methods handled by handle_object_method
        || matches!(method, "keys" | "values" | "has" | "get" | "set" | "len")
        // DateTime methods (from DATETIME_METHODS PHF map)
        || matches!(method, "format")
        // Universal intrinsic methods
        || matches!(method, "type")
    }

    /// Try to track a `Table<T>` type annotation as a DataTable variable.
    ///
    /// If the annotation is `Generic { name: "Table", args: [Reference(T)] }`,
    /// looks up T's schema and marks the variable as `is_datatable`.
    pub(super) fn try_track_datatable_type(
        &mut self,
        type_ann: &shape_ast::ast::TypeAnnotation,
        slot: u16,
        is_local: bool,
    ) -> shape_ast::error::Result<()> {
        use shape_ast::ast::TypeAnnotation;
        if let TypeAnnotation::Generic { name, args } = type_ann {
            if name == "Table" && args.len() == 1 {
                let inner_name = match &args[0] {
                    TypeAnnotation::Reference(t) => Some(t.as_str()),
                    TypeAnnotation::Basic(t) => Some(t.as_str()),
                    _ => None,
                };
                if let Some(type_name) = inner_name {
                    let schema_id = self
                        .type_tracker
                        .schema_registry()
                        .get(type_name)
                        .map(|s| s.id);
                    if let Some(sid) = schema_id {
                        let info = crate::type_tracking::VariableTypeInfo::datatable(
                            sid,
                            type_name.to_string(),
                        );
                        if is_local {
                            self.type_tracker.set_local_type(slot, info);
                        } else {
                            self.type_tracker.set_binding_type(slot, info);
                        }
                    } else if type_name.len() == 1 && type_name.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
                        // Generic type parameter (e.g., T) — skip DataTable tracking,
                        // the concrete type will be determined at the call site.
                    } else {
                        return Err(shape_ast::error::ShapeError::SemanticError {
                            message: format!(
                                "Unknown type '{}' in Table<{}> annotation",
                                type_name, type_name
                            ),
                            location: None,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Check if a variable is a RowView (typed row from Arrow DataTable).
    pub(super) fn is_row_view_variable(&self, name: &str) -> bool {
        if let Some(local_idx) = self.resolve_local(name) {
            if let Some(info) = self.type_tracker.get_local_type(local_idx) {
                return info.is_row_view();
            }
        }
        if let Some(&binding_idx) = self.module_bindings.get(name) {
            if let Some(info) = self.type_tracker.get_binding_type(binding_idx) {
                return info.is_row_view();
            }
        }
        false
    }

    /// Get the available field names for a RowView variable's schema.
    pub(super) fn get_row_view_field_names(&self, name: &str) -> Option<Vec<String>> {
        let type_name = if let Some(local_idx) = self.resolve_local(name) {
            self.type_tracker
                .get_local_type(local_idx)
                .and_then(|info| {
                    if info.is_row_view() {
                        info.type_name.clone()
                    } else {
                        None
                    }
                })
        } else if let Some(&binding_idx) = self.module_bindings.get(name) {
            self.type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| {
                    if info.is_row_view() {
                        info.type_name.clone()
                    } else {
                        None
                    }
                })
        } else {
            None
        };

        if let Some(tn) = type_name {
            if let Some(schema) = self.type_tracker.schema_registry().get(&tn) {
                return Some(schema.field_names().map(|n| n.to_string()).collect());
            }
        }
        None
    }

    /// Try to resolve a property access on a RowView variable to a column ID.
    ///
    /// Returns `Some(col_id)` if the variable is a tracked RowView and the field
    /// exists in its schema. Returns `None` if the variable isn't a RowView or
    /// the field is unknown (caller should emit a compile-time error).
    pub(super) fn try_resolve_row_view_column(
        &self,
        var_name: &str,
        field_name: &str,
    ) -> Option<u32> {
        // Check locals first, then module_bindings
        if let Some(local_idx) = self.resolve_local(var_name) {
            return self
                .type_tracker
                .get_row_view_column_id(local_idx, true, field_name);
        }
        if let Some(&binding_idx) = self.module_bindings.get(var_name) {
            return self
                .type_tracker
                .get_row_view_column_id(binding_idx, false, field_name);
        }
        None
    }

    /// Determine the appropriate LoadCol opcode for a RowView field.
    ///
    /// Looks up the field's FieldType and maps it to the corresponding opcode.
    /// Falls back to LoadColF64 if the type can't be determined.
    pub(super) fn row_view_field_opcode(&self, var_name: &str, field_name: &str) -> OpCode {
        use shape_runtime::type_schema::FieldType;

        let type_name = if let Some(local_idx) = self.resolve_local(var_name) {
            self.type_tracker
                .get_local_type(local_idx)
                .and_then(|info| info.type_name.clone())
        } else if let Some(&binding_idx) = self.module_bindings.get(var_name) {
            self.type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| info.type_name.clone())
        } else {
            None
        };

        if let Some(type_name) = type_name {
            if let Some(schema) = self.type_tracker.schema_registry().get(&type_name) {
                if let Some(field) = schema.get_field(field_name) {
                    return match field.field_type {
                        FieldType::F64 => OpCode::LoadColF64,
                        FieldType::I64 | FieldType::Timestamp => OpCode::LoadColI64,
                        FieldType::Bool => OpCode::LoadColBool,
                        FieldType::String => OpCode::LoadColStr,
                        _ => OpCode::LoadColF64, // default
                    };
                }
            }
        }
        OpCode::LoadColF64 // default
    }

    /// Resolve the NumericType for a RowView field (used for typed opcode emission).
    pub(super) fn resolve_row_view_field_numeric_type(
        &self,
        var_name: &str,
        field_name: &str,
    ) -> Option<crate::type_tracking::NumericType> {
        use crate::type_tracking::NumericType;
        use shape_runtime::type_schema::FieldType;

        let type_name = if let Some(local_idx) = self.resolve_local(var_name) {
            self.type_tracker
                .get_local_type(local_idx)
                .and_then(|info| info.type_name.clone())
        } else if let Some(&binding_idx) = self.module_bindings.get(var_name) {
            self.type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| info.type_name.clone())
        } else {
            None
        };

        if let Some(type_name) = type_name {
            if let Some(schema) = self.type_tracker.schema_registry().get(&type_name) {
                if let Some(field) = schema.get_field(field_name) {
                    return match field.field_type {
                        FieldType::F64 => Some(NumericType::Number),
                        FieldType::I64 | FieldType::Timestamp => Some(NumericType::Int),
                        FieldType::Decimal => Some(NumericType::Decimal),
                        _ => None,
                    };
                }
            }
        }
        None
    }

    /// Convert a TypeAnnotation to a FieldType for TypeSchema registration
    pub(super) fn type_annotation_to_field_type(
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> shape_runtime::type_schema::FieldType {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_schema::FieldType;
        match ann {
            TypeAnnotation::Basic(s) => match s.as_str() {
                "number" | "float" | "f64" | "f32" => FieldType::F64,
                "i8" => FieldType::I8,
                "u8" => FieldType::U8,
                "i16" => FieldType::I16,
                "u16" => FieldType::U16,
                "i32" => FieldType::I32,
                "u32" => FieldType::U32,
                "u64" => FieldType::U64,
                "int" | "i64" | "integer" | "isize" | "usize" | "byte" | "char" => FieldType::I64,
                "string" | "str" => FieldType::String,
                "decimal" => FieldType::Decimal,
                "bool" | "boolean" => FieldType::Bool,
                "timestamp" => FieldType::Timestamp,
                // Non-primitive type names (e.g. "Server", "Inner") are nested
                // object references.  The parser emits Basic for `ident` matches
                // inside `basic_type`, so treat unknown names as Object references
                // to enable typed field access on nested structs.
                other => FieldType::Object(other.to_string()),
            },
            TypeAnnotation::Reference(s) => FieldType::Object(s.to_string()),
            TypeAnnotation::Array(inner) => {
                FieldType::Array(Box::new(Self::type_annotation_to_field_type(inner)))
            }
            TypeAnnotation::Generic { name, .. } => match name.as_str() {
                // Generic containers that need NaN boxing
                "HashMap" | "Map" | "Result" | "Option" | "Set" => FieldType::Any,
                // User-defined generic structs — preserve the type name
                other => FieldType::Object(other.to_string()),
            },
            _ => FieldType::Any,
        }
    }

    /// Evaluate an annotation argument expression to a string representation.
    /// Only handles compile-time evaluable expressions (literals).
    pub(super) fn eval_annotation_arg(expr: &shape_ast::ast::Expr) -> Option<String> {
        use shape_ast::ast::{Expr, Literal};
        match expr {
            Expr::Literal(Literal::String(s), _) => Some(s.clone()),
            Expr::Literal(Literal::Number(n), _) => Some(n.to_string()),
            Expr::Literal(Literal::Int(i), _) => Some(i.to_string()),
            Expr::Literal(Literal::Bool(b), _) => Some(b.to_string()),
            _ => None,
        }
    }

    /// Get the schema ID for a `Table<T>` type annotation, if applicable.
    ///
    /// Returns `Some(schema_id)` if the annotation is `Table<T>` and `T` is a registered
    /// TypeSchema. Returns `None` otherwise.
    pub(super) fn get_table_schema_id(
        &self,
        type_ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<u16> {
        use shape_ast::ast::TypeAnnotation;
        if let TypeAnnotation::Generic { name, args } = type_ann {
            if name == "Table" && args.len() == 1 {
                let inner_name = match &args[0] {
                    TypeAnnotation::Basic(t) => Some(t.as_str()),
                    TypeAnnotation::Reference(t) => Some(t.as_str()),
                    _ => None,
                };
                if let Some(type_name) = inner_name {
                    return self
                        .type_tracker
                        .schema_registry()
                        .get(type_name)
                        .map(|s| s.id as u16);
                }
            }
        }
        None
    }

    // ===== Drop scope management =====

    /// Push a new drop scope. Must be paired with pop_drop_scope().
    pub(super) fn push_drop_scope(&mut self) {
        self.drop_locals.push(Vec::new());
    }

    /// Pop the current drop scope, emitting DropCall instructions for all
    /// tracked locals in reverse order.
    pub(super) fn pop_drop_scope(&mut self) -> Result<()> {
        // Emit DropCall for each tracked local in reverse order
        if let Some(locals) = self.drop_locals.pop() {
            for (local_idx, is_async) in locals.into_iter().rev() {
                self.emit_drop_call_for_local(local_idx, is_async);
            }
        }
        Ok(())
    }

    /// Emit a single LoadLocal + DropCall pair for a local variable.
    /// The type name is resolved from the type tracker and encoded as a
    /// Property operand so the executor can look up `TypeName::drop`.
    fn emit_drop_call_for_local(&mut self, local_idx: u16, is_async: bool) {
        let type_name_opt = self
            .type_tracker
            .get_local_type(local_idx)
            .and_then(|info| info.type_name.clone());
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(local_idx)),
        ));
        let opcode = if is_async {
            OpCode::DropCallAsync
        } else {
            OpCode::DropCall
        };
        if let Some(type_name) = type_name_opt {
            let str_idx = self.program.add_string(type_name);
            self.emit(Instruction::new(opcode, Some(Operand::Property(str_idx))));
        } else {
            self.emit(Instruction::simple(opcode));
        }
    }

    /// Emit a single LoadModuleBinding + DropCall pair for a module binding.
    /// Similar to `emit_drop_call_for_local` but loads from module bindings.
    pub(super) fn emit_drop_call_for_module_binding(&mut self, binding_idx: u16, is_async: bool) {
        let type_name_opt = self
            .type_tracker
            .get_binding_type(binding_idx)
            .and_then(|info| info.type_name.clone());
        self.emit(Instruction::new(
            OpCode::LoadModuleBinding,
            Some(Operand::ModuleBinding(binding_idx)),
        ));
        let opcode = if is_async {
            OpCode::DropCallAsync
        } else {
            OpCode::DropCall
        };
        if let Some(type_name) = type_name_opt {
            let str_idx = self.program.add_string(type_name);
            self.emit(Instruction::new(opcode, Some(Operand::Property(str_idx))));
        } else {
            self.emit(Instruction::simple(opcode));
        }
    }

    /// Track a local variable as needing Drop at scope exit.
    pub(super) fn track_drop_local(&mut self, local_idx: u16, is_async: bool) {
        if let Some(scope) = self.drop_locals.last_mut() {
            scope.push((local_idx, is_async));
        }
    }

    /// Resolve the DropKind for a local variable's type.
    /// Returns None if the type is unknown or has no Drop impl.
    pub(super) fn local_drop_kind(&self, local_idx: u16) -> Option<DropKind> {
        let type_name = self
            .type_tracker
            .get_local_type(local_idx)
            .and_then(|info| info.type_name.as_ref())?;
        self.drop_type_info.get(type_name).copied()
    }

    /// Resolve DropKind from a type annotation.
    pub(super) fn annotation_drop_kind(&self, type_ann: &TypeAnnotation) -> Option<DropKind> {
        let type_name = Self::tracked_type_name_from_annotation(type_ann)?;
        self.drop_type_info.get(&type_name).copied()
    }

    /// Emit drops for all scopes being exited (used by return/break/continue).
    /// `scopes_to_exit` is the number of drop scopes to emit drops for.
    pub(super) fn emit_drops_for_early_exit(&mut self, scopes_to_exit: usize) -> Result<()> {
        let total = self.drop_locals.len();
        if scopes_to_exit > total {
            return Ok(());
        }
        // Collect locals from scopes being exited (innermost first)
        let mut scopes: Vec<Vec<(u16, bool)>> = Vec::new();
        for i in (total - scopes_to_exit..total).rev() {
            let locals = self.drop_locals.get(i).cloned().unwrap_or_default();
            scopes.push(locals);
        }
        // Now emit DropCall instructions
        for locals in scopes {
            for (local_idx, is_async) in locals.into_iter().rev() {
                self.emit_drop_call_for_local(local_idx, is_async);
            }
        }
        Ok(())
    }

    /// Track a module binding as needing Drop at program exit.
    pub(super) fn track_drop_module_binding(&mut self, binding_idx: u16, is_async: bool) {
        self.drop_module_bindings.push((binding_idx, is_async));
    }
}


#[cfg(test)]
mod tests {
    use super::super::BytecodeCompiler;
    use crate::compiler::ParamPassMode;
    use crate::type_tracking::BindingStorageClass;
    use shape_ast::ast::{Expr, Span, TypeAnnotation};
    use shape_runtime::type_schema::FieldType;

    #[test]
    fn test_type_annotation_to_field_type_array_recursive() {
        let ann = TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("int".to_string())));
        let ft = BytecodeCompiler::type_annotation_to_field_type(&ann);
        assert_eq!(ft, FieldType::Array(Box::new(FieldType::I64)));
    }

    #[test]
    fn test_type_annotation_to_field_type_optional() {
        let ann = TypeAnnotation::Generic {
            name: "Option".into(),
            args: vec![TypeAnnotation::Basic("int".to_string())],
        };
        let ft = BytecodeCompiler::type_annotation_to_field_type(&ann);
        assert_eq!(ft, FieldType::Any);
    }

    #[test]
    fn test_type_annotation_to_field_type_generic_hashmap() {
        let ann = TypeAnnotation::Generic {
            name: "HashMap".into(),
            args: vec![
                TypeAnnotation::Basic("string".to_string()),
                TypeAnnotation::Basic("int".to_string()),
            ],
        };
        let ft = BytecodeCompiler::type_annotation_to_field_type(&ann);
        assert_eq!(ft, FieldType::Any);
    }

    #[test]
    fn test_type_annotation_to_field_type_generic_user_struct() {
        let ann = TypeAnnotation::Generic {
            name: "MyContainer".into(),
            args: vec![TypeAnnotation::Basic("string".to_string())],
        };
        let ft = BytecodeCompiler::type_annotation_to_field_type(&ann);
        assert_eq!(ft, FieldType::Object("MyContainer".to_string()));
    }

    #[test]
    fn test_flexible_storage_promotion_is_monotonic() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        compiler.promote_flexible_binding_storage_for_slot(
            slot,
            true,
            BindingStorageClass::UniqueHeap,
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );

        compiler.promote_flexible_binding_storage_for_slot(slot, true, BindingStorageClass::Direct);
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );

        compiler.promote_flexible_binding_storage_for_slot(
            slot,
            true,
            BindingStorageClass::SharedCow,
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::SharedCow)
        );
    }

    #[test]
    fn test_escape_planner_marks_array_element_identifier_as_unique_heap() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        let expr = Expr::Array(
            vec![Expr::Identifier("value".to_string(), Span::DUMMY)],
            Span::DUMMY,
        );
        compiler.plan_flexible_binding_escape_from_expr(&expr);

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    #[test]
    fn test_escape_planner_marks_if_branch_identifier_as_unique_heap() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        let expr = Expr::If(
            Box::new(shape_ast::ast::IfExpr {
                condition: Box::new(Expr::Literal(
                    shape_ast::ast::Literal::Bool(true),
                    Span::DUMMY,
                )),
                then_branch: Box::new(Expr::Identifier("value".to_string(), Span::DUMMY)),
                else_branch: None,
            }),
            Span::DUMMY,
        );
        compiler.plan_flexible_binding_escape_from_expr(&expr);

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    #[test]
    fn test_escape_planner_marks_async_let_rhs_identifier_as_unique_heap() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        let expr = Expr::AsyncLet(
            Box::new(shape_ast::ast::AsyncLetExpr {
                name: "task".to_string(),
                expr: Box::new(Expr::Identifier("value".to_string(), Span::DUMMY)),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        );
        compiler.plan_flexible_binding_escape_from_expr(&expr);

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    #[test]
    fn test_call_args_mark_by_value_identifier_as_unique_heap() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        compiler
            .compile_call_args(&[Expr::Identifier("value".to_string(), Span::DUMMY)], None)
            .expect("call args should compile");

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    #[test]
    fn test_call_args_leave_by_ref_identifier_storage_unchanged() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        compiler
            .compile_call_args(
                &[Expr::Identifier("value".to_string(), Span::DUMMY)],
                Some(&[ParamPassMode::ByRefShared]),
            )
            .expect("reference call args should compile");

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::Deferred)
        );
    }
}
