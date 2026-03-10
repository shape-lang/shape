//! Function and method call expression compilation

use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::compiler::string_interpolation::has_interpolation;
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::{NumericType, VariableKind, VariableTypeInfo};
use shape_ast::ast::{BinaryOp, Expr, InterpolationMode, Literal, Span, Spanned, UnaryOp};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_system::suggestions::suggest_function;
use shape_runtime::type_system::{BuiltinTypes, Type};
use shape_value::ValueWord;
use std::sync::Arc;

use super::super::BytecodeCompiler;

/// Map a return type name string to a NumericType.
fn return_type_to_numeric(type_name: &str) -> Option<NumericType> {
    if BuiltinTypes::is_integer_type_name(type_name) {
        return Some(NumericType::Int);
    }
    if BuiltinTypes::is_number_type_name(type_name) {
        return Some(NumericType::Number);
    }
    match type_name {
        "decimal" | "Decimal" => Some(NumericType::Decimal),
        _ => None,
    }
}

/// Get the known return NumericType for a builtin function name.
fn builtin_return_numeric_type(name: &str) -> Option<NumericType> {
    match name {
        // Int-returning builtins
        "len" | "count" => Some(NumericType::Int),
        // Number-returning builtins
        "abs" | "sqrt" | "ceil" | "floor" | "round" | "sum" | "mean" | "min" | "max" | "sin"
        | "cos" | "tan" | "exp" | "ln" | "log" | "stddev" | "std" | "variance" => {
            Some(NumericType::Number)
        }
        _ => None,
    }
}

/// Get the known return NumericType for a method name.
fn method_return_numeric_type(method: &str) -> Option<NumericType> {
    match method {
        // Int-returning methods
        "len" | "length" | "count" | "indexOf" | "findIndex" => Some(NumericType::Int),
        // Number-returning methods
        "sum" | "mean" | "avg" | "min" | "max" | "std" | "var" | "abs" | "sqrt" => {
            Some(NumericType::Number)
        }
        _ => None,
    }
}

/// Conservative compile-time-constant check for const parameters.
/// Accepts literals and recursively literal-composed containers.
fn is_compile_time_const_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_, _) => true,
        Expr::UnaryOp { operand, .. } => is_compile_time_const_expr(operand),
        Expr::BinaryOp { left, right, .. } => {
            is_compile_time_const_expr(left) && is_compile_time_const_expr(right)
        }
        Expr::Array(items, _) => items.iter().all(is_compile_time_const_expr),
        Expr::Object(entries, _) => entries
            .iter()
            .all(|entry| matches!(entry, shape_ast::ast::ObjectEntry::Field { value, .. } if is_compile_time_const_expr(value))),
        _ => false,
    }
}

fn literal_to_nanboxed(literal: &Literal) -> Option<ValueWord> {
    match literal {
        Literal::Int(i) => Some(ValueWord::from_i64(*i)),
        Literal::UInt(u) => Some(ValueWord::from_native_u64(*u)),
        Literal::TypedInt(v, _) => Some(ValueWord::from_i64(*v)),
        Literal::Number(n) => Some(ValueWord::from_f64(*n)),
        Literal::Decimal(d) => Some(ValueWord::from_decimal(*d)),
        Literal::String(s) => Some(ValueWord::from_string(Arc::new(s.clone()))),
        Literal::Char(c) => Some(ValueWord::from_char(*c)),
        Literal::FormattedString { value, .. } => {
            Some(ValueWord::from_string(Arc::new(value.clone())))
        }
        Literal::ContentString { value, .. } => {
            Some(ValueWord::from_string(Arc::new(value.clone())))
        }
        Literal::Bool(b) => Some(ValueWord::from_bool(*b)),
        Literal::None => Some(ValueWord::none()),
        Literal::Unit => Some(ValueWord::unit()),
        Literal::Timeframe(_) => None,
    }
}

pub(crate) fn eval_const_expr_to_nanboxed(expr: &Expr) -> Option<ValueWord> {
    match expr {
        Expr::Literal(literal, _) => literal_to_nanboxed(literal),
        Expr::Array(items, _) => {
            let values: Vec<ValueWord> = items
                .iter()
                .map(eval_const_expr_to_nanboxed)
                .collect::<Option<Vec<_>>>()?;
            Some(ValueWord::from_array(Arc::new(values)))
        }
        Expr::UnaryOp { op, operand, .. } => {
            let value = eval_const_expr_to_nanboxed(operand)?;
            match op {
                UnaryOp::Neg => {
                    if let Some(i) = value.as_i64() {
                        Some(ValueWord::from_i64(-i))
                    } else if let Some(n) = value.as_f64() {
                        Some(ValueWord::from_f64(-n))
                    } else {
                        None
                    }
                }
                UnaryOp::Not => value.as_bool().map(|b| ValueWord::from_bool(!b)),
                UnaryOp::BitNot => value.as_i64().map(|i| ValueWord::from_i64(!i)),
            }
        }
        Expr::BinaryOp {
            left, op, right, ..
        } => {
            let lhs = eval_const_expr_to_nanboxed(left)?;
            let rhs = eval_const_expr_to_nanboxed(right)?;
            match op {
                BinaryOp::Add => {
                    if let (Some(a), Some(b)) = (lhs.as_i64(), rhs.as_i64()) {
                        Some(ValueWord::from_i64(a + b))
                    } else if let (Some(a), Some(b)) = (lhs.as_decimal(), rhs.as_decimal()) {
                        Some(ValueWord::from_decimal(a + b))
                    } else if let (Some(a), Some(b)) = (lhs.as_f64(), rhs.as_f64()) {
                        Some(ValueWord::from_f64(a + b))
                    } else if let (Some(a), Some(b)) = (lhs.as_str(), rhs.as_str()) {
                        Some(ValueWord::from_string(Arc::new(format!("{}{}", a, b))))
                    } else {
                        None
                    }
                }
                BinaryOp::Sub => {
                    if let (Some(a), Some(b)) = (lhs.as_i64(), rhs.as_i64()) {
                        Some(ValueWord::from_i64(a - b))
                    } else if let (Some(a), Some(b)) = (lhs.as_decimal(), rhs.as_decimal()) {
                        Some(ValueWord::from_decimal(a - b))
                    } else if let (Some(a), Some(b)) = (lhs.as_f64(), rhs.as_f64()) {
                        Some(ValueWord::from_f64(a - b))
                    } else {
                        None
                    }
                }
                BinaryOp::Mul => {
                    if let (Some(a), Some(b)) = (lhs.as_i64(), rhs.as_i64()) {
                        Some(ValueWord::from_i64(a * b))
                    } else if let (Some(a), Some(b)) = (lhs.as_decimal(), rhs.as_decimal()) {
                        Some(ValueWord::from_decimal(a * b))
                    } else if let (Some(a), Some(b)) = (lhs.as_f64(), rhs.as_f64()) {
                        Some(ValueWord::from_f64(a * b))
                    } else {
                        None
                    }
                }
                BinaryOp::Div => {
                    if let (Some(a), Some(b)) = (lhs.as_f64(), rhs.as_f64()) {
                        Some(ValueWord::from_f64(a / b))
                    } else {
                        None
                    }
                }
                BinaryOp::Mod => {
                    if let (Some(a), Some(b)) = (lhs.as_i64(), rhs.as_i64()) {
                        Some(ValueWord::from_i64(a % b))
                    } else if let (Some(a), Some(b)) = (lhs.as_f64(), rhs.as_f64()) {
                        Some(ValueWord::from_f64(a % b))
                    } else {
                        None
                    }
                }
                BinaryOp::Pow => {
                    if let (Some(a), Some(b)) = (lhs.as_f64(), rhs.as_f64()) {
                        Some(ValueWord::from_f64(a.powf(b)))
                    } else {
                        None
                    }
                }
                BinaryOp::And => Some(ValueWord::from_bool(lhs.as_bool()? && rhs.as_bool()?)),
                BinaryOp::Or => Some(ValueWord::from_bool(lhs.as_bool()? || rhs.as_bool()?)),
                BinaryOp::Equal => Some(ValueWord::from_bool(lhs.clone() == rhs.clone())),
                BinaryOp::NotEqual => Some(ValueWord::from_bool(lhs.clone() != rhs.clone())),
                BinaryOp::BitAnd => Some(ValueWord::from_i64(lhs.as_i64()? & rhs.as_i64()?)),
                BinaryOp::BitOr => Some(ValueWord::from_i64(lhs.as_i64()? | rhs.as_i64()?)),
                BinaryOp::BitXor => Some(ValueWord::from_i64(lhs.as_i64()? ^ rhs.as_i64()?)),
                BinaryOp::BitShl => Some(ValueWord::from_i64(lhs.as_i64()? << rhs.as_i64()?)),
                BinaryOp::BitShr => Some(ValueWord::from_i64(lhs.as_i64()? >> rhs.as_i64()?)),
                BinaryOp::NullCoalesce => {
                    if lhs.is_none() {
                        Some(rhs)
                    } else {
                        Some(lhs)
                    }
                }
                _ => None,
            }
        }
        // Object consts are intentionally rejected here until object const
        // specialization values are represented without anonymous runtime schemas.
        Expr::Object(_, _) => None,
        _ => None,
    }
}

fn const_expr_fingerprint(expr: &Expr) -> Option<String> {
    let value = eval_const_expr_to_nanboxed(expr)?;
    Some(format!("{:?}", value.clone()))
}

impl BytecodeCompiler {
    fn extract_table_schema_from_annotation(
        &mut self,
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<(u32, String)> {
        let shape_ast::ast::TypeAnnotation::Generic { name, args } = ann else {
            return None;
        };
        if name != "Table" || args.len() != 1 {
            return None;
        }

        match &args[0] {
            shape_ast::ast::TypeAnnotation::Reference(name)
            | shape_ast::ast::TypeAnnotation::Basic(name) => self
                .type_tracker
                .schema_registry()
                .get(name)
                .map(|schema| (schema.id, name.clone())),
            shape_ast::ast::TypeAnnotation::Object(fields) => {
                let field_refs: Vec<&str> =
                    fields.iter().map(|field| field.name.as_str()).collect();
                let schema_id = self.type_tracker.register_inline_object_schema(&field_refs);
                let schema_name = self
                    .type_tracker
                    .schema_registry()
                    .get_by_id(schema_id)
                    .map(|schema| schema.name.clone())
                    .unwrap_or_else(|| format!("__anon_{}", schema_id));
                Some((schema_id, schema_name))
            }
            _ => None,
        }
    }

    fn extract_object_schema_id_from_annotation(
        &mut self,
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<u32> {
        let shape_ast::ast::TypeAnnotation::Object(fields) = ann else {
            return None;
        };
        let field_refs: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
        let schema_id = self.type_tracker.register_inline_object_schema(&field_refs);
        let mut map = std::collections::HashMap::with_capacity(fields.len());
        for field in fields {
            map.insert(field.name.clone(), field.type_annotation.clone());
        }
        self.type_tracker
            .register_object_field_contracts(schema_id, map);
        Some(schema_id)
    }

    fn type_info_from_annotation(
        &mut self,
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<VariableTypeInfo> {
        match ann {
            shape_ast::ast::TypeAnnotation::Generic { name, .. } if name == "Table" => self
                .extract_table_schema_from_annotation(ann)
                .map(|(schema_id, type_name)| VariableTypeInfo::datatable(schema_id, type_name)),
            shape_ast::ast::TypeAnnotation::Object(_) => {
                let schema_id = self.extract_object_schema_id_from_annotation(ann)?;
                let schema_name = self
                    .type_tracker
                    .schema_registry()
                    .get_by_id(schema_id)
                    .map(|schema| schema.name.clone())
                    .unwrap_or_else(|| format!("__anon_{}", schema_id));
                Some(VariableTypeInfo::known(schema_id, schema_name))
            }
            shape_ast::ast::TypeAnnotation::Reference(name)
            | shape_ast::ast::TypeAnnotation::Basic(name) => self
                .type_tracker
                .schema_registry()
                .get(name)
                .map(|schema| VariableTypeInfo::known(schema.id, name.clone())),
            _ => None,
        }
    }

    fn type_info_from_inferred_type(&mut self, inferred: &Type) -> Option<VariableTypeInfo> {
        let ann = inferred.to_annotation()?;
        self.type_info_from_annotation(&ann)
    }

    fn table_schema_from_type_info(type_info: &VariableTypeInfo) -> Option<(u32, String)> {
        if type_info.is_datatable() {
            Some((type_info.schema_id?, type_info.type_name.clone()?))
        } else {
            None
        }
    }

    fn value_schema_from_type_info(type_info: &VariableTypeInfo) -> Option<u32> {
        if matches!(type_info.kind, VariableKind::Value) {
            type_info.schema_id
        } else {
            None
        }
    }

    fn extract_table_schema_from_callable_field(
        &mut self,
        receiver_schema_id: u32,
        field_name: &str,
    ) -> Option<(u32, String)> {
        let field_ann = self
            .type_tracker
            .get_object_field_contract(receiver_schema_id, field_name)?
            .clone();
        let shape_ast::ast::TypeAnnotation::Function { params, returns } = field_ann else {
            return None;
        };
        if !params.is_empty() {
            return None;
        }
        self.extract_table_schema_from_annotation(&returns)
    }

    fn is_native_module_export(&self, module_name: &str, export_name: &str) -> bool {
        self.extension_registry
            .as_ref()
            .and_then(|registry| registry.iter().rev().find(|m| m.name == module_name))
            .is_some_and(|module| module.has_export(export_name))
    }

    fn is_native_module_export_available(&self, module_name: &str, export_name: &str) -> bool {
        self.extension_registry
            .as_ref()
            .and_then(|registry| registry.iter().rev().find(|m| m.name == module_name))
            .is_some_and(|module| module.is_export_available(export_name, self.comptime_mode))
    }

    fn ensure_const_specialization(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Result<Option<(String, usize)>> {
        let Some(const_param_indices) = self.function_const_params.get(name).cloned() else {
            return Ok(None);
        };
        if const_param_indices.is_empty() {
            return Ok(None);
        }

        let template_def =
            self.function_defs
                .get(name)
                .cloned()
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "Missing function template definition for const specialization: '{}'",
                        name
                    ),
                    location: None,
                })?;
        let has_comptime_handlers = template_def.annotations.iter().any(|ann| {
            self.program
                .compiled_annotations
                .get(&ann.name)
                .map(|compiled| {
                    compiled.comptime_pre_handler.is_some()
                        || compiled.comptime_post_handler.is_some()
                })
                .unwrap_or(false)
        });
        if !has_comptime_handlers {
            return Ok(None);
        }

        let mut key_parts: Vec<String> = Vec::new();
        let mut const_bindings: Vec<(String, ValueWord)> = Vec::new();

        for param_idx in const_param_indices {
            let param =
                template_def
                    .params
                    .get(param_idx)
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: format!(
                            "Invalid const parameter index {} for function '{}'",
                            param_idx, name
                        ),
                        location: None,
                    })?;
            let param_name = param
                .simple_name()
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "Const parameter #{} in '{}' must use an identifier pattern",
                        param_idx + 1,
                        name
                    ),
                    location: Some(self.span_to_source_location(param.span())),
                })?;

            let (expr, span) = if let Some(arg) = args.get(param_idx) {
                (arg, arg.span())
            } else if let Some(default_expr) = &param.default_value {
                (default_expr, default_expr.span())
            } else {
                continue;
            };

            let value = eval_const_expr_to_nanboxed(expr).ok_or_else(|| ShapeError::SemanticError {
                message: format!(
                    "Function '{}' const parameter '{}' must be a literal-evaluable compile-time expression",
                    name, param_name
                ),
                location: Some(self.span_to_source_location(span)),
            })?;
            let fingerprint =
                const_expr_fingerprint(expr).ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "Function '{}' const parameter '{}' could not be fingerprinted",
                        name, param_name
                    ),
                    location: Some(self.span_to_source_location(span)),
                })?;
            key_parts.push(format!("{}={}", param_name, fingerprint));
            const_bindings.push((param_name.to_string(), value));
        }

        if key_parts.is_empty() {
            return Ok(None);
        }

        let specialization_key = format!("{}::{}", name, key_parts.join("|"));
        if let Some(existing_idx) = self.const_specializations.get(&specialization_key).copied() {
            let existing_name = self.program.functions[existing_idx].name.clone();
            return Ok(Some((existing_name, existing_idx)));
        }

        let specialization_name = format!("{}__const_{}", name, self.next_const_specialization_id);
        self.next_const_specialization_id += 1;

        let mut specialized_def = template_def;
        specialized_def.name = specialization_name.clone();

        self.specialization_const_bindings
            .insert(specialization_name.clone(), const_bindings);
        self.register_function(&specialized_def)?;
        let specialization_idx =
            self.find_function(&specialization_name)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "Failed to register const specialization function '{}'",
                        specialization_name
                    ),
                    location: None,
                })?;
        self.const_specializations
            .insert(specialization_key, specialization_idx);

        if let Err(err) = self.compile_function(&specialized_def) {
            self.specialization_const_bindings
                .remove(&specialization_name);
            return Err(err);
        }

        self.specialization_const_bindings
            .remove(&specialization_name);
        Ok(Some((specialization_name, specialization_idx)))
    }

    /// Compile a function call expression
    pub(super) fn compile_expr_function_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<()> {
        if name == "type_info" {
            return Err(ShapeError::SemanticError {
                message: "type_info has been removed. Use `<expr>.type()` for static type queries."
                    .to_string(),
                location: Some(self.span_to_source_location(span)),
            });
        }

        // Reject comptime-only builtins outside of comptime blocks.
        // These functions are only available inside `comptime { }` blocks.
        if Self::is_comptime_only_builtin(name) && !self.comptime_mode {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "'{}' is a comptime-only builtin and can only be called inside a `comptime {{ }}` block",
                    name
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        // Check locals FIRST — function parameters (and other local variables holding
        // callable values) must take priority over global function lookup.  Without this,
        // `fn apply(f, x) { f(x) }` would fail because `find_function("f")` returns None
        // and the code falls through to "Undefined function" error.
        if self.resolve_local(name).is_some()
            || self.mutable_closure_captures.contains_key(name)
            || self.resolve_scoped_module_binding_name(name).is_some()
        {
            // Use compile_expr_identifier to correctly load the callee value,
            // handling ref_locals (DerefLoad), mutable closure captures (LoadClosure), etc.
            self.compile_expr_identifier(name, span)?;

            let writebacks = self.compile_call_args(args, None)?;
            let arg_count = self
                .program
                .add_constant(Constant::Number(args.len() as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));
            self.emit(Instruction::simple(OpCode::CallValue));
            if !writebacks.is_empty() {
                let result_local = self.declare_temp_local("__call_value_result_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));
                for (shadow_local, binding_idx) in writebacks {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(shadow_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
            }
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.last_expr_numeric_type = None;
            return Ok(());
        }

        // Check for user-defined functions (after locals — function parameters take priority)
        if let Some(func_idx) = self.find_function(name) {
            let resolved_name = self.program.functions[func_idx].name.clone();
            let is_comptime_fn = self
                .function_defs
                .get(&resolved_name)
                .or_else(|| self.function_defs.get(name))
                .map(|def| def.is_comptime)
                .unwrap_or(false);
            if is_comptime_fn && !self.comptime_mode {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "'{}' is declared as `comptime fn` and can only be called from comptime contexts",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }

            let mut call_name = resolved_name;
            let mut call_func_idx = func_idx;

            let total_arity = self.program.functions[call_func_idx].arity as usize;
            let (required_arity, effective_total_arity) = self
                .function_arity_bounds
                .get(&call_name)
                .copied()
                .unwrap_or((total_arity, total_arity));
            let actual_arity = args.len();
            if actual_arity < required_arity || actual_arity > effective_total_arity {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Function '{}' expects between {} and {} arguments, got {}",
                        name, required_arity, effective_total_arity, actual_arity
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }

            if let Some(const_param_indices) = self.function_const_params.get(&call_name).cloned() {
                for idx in const_param_indices {
                    if idx >= actual_arity {
                        continue;
                    }
                    let arg = &args[idx];
                    if !is_compile_time_const_expr(arg) {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "Function '{}' parameter #{} is declared `const` and requires a compile-time constant argument",
                                name,
                                idx + 1
                            ),
                            location: Some(self.span_to_source_location(arg.span())),
                        });
                    }
                }

                if let Some((specialized_name, specialized_idx)) =
                    self.ensure_const_specialization(&call_name, args)?
                {
                    call_name = specialized_name;
                    call_func_idx = specialized_idx;
                }
            }

            let ref_params = self.program.functions[call_func_idx].ref_params.clone();
            let ref_mutates = self.program.functions[call_func_idx].ref_mutates.clone();
            let pass_modes = Self::pass_modes_from_ref_flags(&ref_params, &ref_mutates);
            let writebacks = self.compile_call_args(args, Some(&pass_modes))?;

            // Compile default expressions for missing arguments
            if actual_arity < effective_total_arity {
                let func_def = self
                    .function_defs
                    .get(&call_name)
                    .or_else(|| self.function_defs.get(name))
                    .cloned();
                for param_idx in actual_arity..effective_total_arity {
                    let mut emitted_default = false;
                    if let Some(ref fdef) = func_def {
                        if let Some(param) = fdef.params.get(param_idx) {
                            if let Some(ref default_expr) = param.default_value {
                                let is_ref_param =
                                    ref_params.get(param_idx).copied().unwrap_or(false);
                                let default_clone = default_expr.clone();
                                self.compile_expr(&default_clone)?;
                                // If the callee expects a reference, wrap the
                                // default value: store in a temp and MakeRef.
                                if is_ref_param {
                                    let temp = self.declare_temp_local("__default_ref_")?;
                                    self.emit(Instruction::new(
                                        OpCode::StoreLocal,
                                        Some(Operand::Local(temp)),
                                    ));
                                    self.emit(Instruction::new(
                                        OpCode::MakeRef,
                                        Some(Operand::Local(temp)),
                                    ));
                                }
                                emitted_default = true;
                            }
                        }
                    }
                    if !emitted_default {
                        self.emit_unit();
                    }
                }
            }
            let arg_count = self
                .program
                .add_constant(Constant::Number(effective_total_arity as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));
            self.emit(Instruction::new(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(
                    call_func_idx as u16,
                ))),
            ));
            // Record callee as a blob dependency
            if let Some(ref mut blob) = self.current_blob_builder {
                blob.record_call(&call_name);
            }
            if !writebacks.is_empty() {
                let result_local = self.declare_temp_local("__call_result_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));
                for (shadow_local, binding_idx) in writebacks {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(shadow_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
            }

            let return_type_annotation = self
                .function_defs
                .get(&call_name)
                .and_then(|def| def.return_type.clone())
                .or_else(|| {
                    self.foreign_function_defs
                        .get(&call_name)
                        .and_then(|def| def.return_type.clone())
                });
            self.last_expr_type_info = return_type_annotation
                .as_ref()
                .and_then(|ann| self.type_info_from_annotation(ann));
            self.last_expr_schema = self
                .last_expr_type_info
                .as_ref()
                .and_then(Self::value_schema_from_type_info);

            // Propagate return type for typed opcode emission
            self.last_expr_numeric_type = self
                .type_tracker
                .get_function_return_type(&call_name)
                .and_then(|rt| return_type_to_numeric(rt));
            return Ok(());
        }

        // Builtins take precedence - they're optimized Rust implementations
        if let Some(builtin) = self.get_builtin_function(name) {
            // Special handling for print with string interpolation
            if builtin == BuiltinFunction::Print {
                return self.compile_print_with_interpolation(args);
            }

            for arg in args {
                self.compile_expr_as_value_or_placeholder(arg)?;
            }
            if self.builtin_requires_arg_count(builtin) {
                let arg_count = self
                    .program
                    .add_constant(Constant::Number(args.len() as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(arg_count)),
                ));
            }
            self.emit(Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(builtin)),
            ));
            // Propagate known return type for builtin functions
            self.last_expr_numeric_type = builtin_return_numeric_type(name);
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            return Ok(());
        }

        // Removed global data-loading API:
        // load("provider", { ... }) -> provider.load({ ... }) (module-scoped).
        if name == "load"
            && args.len() == 2
            && matches!(args[0], Expr::Literal(Literal::String(_), _))
        {
            return Err(ShapeError::SemanticError {
                message:
                    "load(provider, params) has been removed. Use module-scoped calls like `provider.load({ ... })`."
                        .to_string(),
                location: Some(self.span_to_source_location(span)),
            });
        }

        // Named import from a native extension module (e.g. `from file use { read_text }`).
        // Native modules have no AST to inline, so the function won't be in program.functions.
        // Rewrite the call as a module namespace call (e.g. file.read_text(...)).
        if let Some(imported) = self.imported_names.get(name).cloned() {
            if self.is_native_module_export(&imported.module_path, &imported.original_name) {
                // Ensure the module has a namespace binding (auto-create if needed)
                if !self
                    .module_namespace_bindings
                    .contains(&imported.module_path)
                {
                    let binding_idx = self.get_or_create_module_binding(&imported.module_path);
                    self.module_namespace_bindings
                        .insert(imported.module_path.clone());
                    self.register_extension_module_schema(&imported.module_path);
                    let module_schema_name = format!("__mod_{}", imported.module_path);
                    if self
                        .type_tracker
                        .schema_registry()
                        .get(&module_schema_name)
                        .is_some()
                    {
                        self.set_module_binding_type_info(binding_idx, &module_schema_name);
                    }
                }
                let receiver = Expr::Identifier(imported.module_path.clone(), span);
                return self.compile_module_namespace_call(
                    &receiver,
                    &imported.original_name,
                    args,
                );
            }
        }

        // Build error message with suggestions
        let mut message = format!("Undefined function: {}", name);

        // Try import suggestion first
        if let Some(module_path) = self.suggest_import(name) {
            message = format!(
                "Unknown function '{}'. Did you mean to import it via '{}'\n\n  from {} use {{ {} }}",
                name, module_path, module_path, name
            );
        } else {
            // Try typo suggestion from available function names
            let available = self.collect_available_function_names();
            if let Some(suggestion) = suggest_function(name, &available) {
                message.push_str(&format!(". {}", suggestion));
            }
        }
        Err(ShapeError::RuntimeError {
            message,
            location: Some(self.span_to_source_location(span)),
        })
    }

    /// Check if a method name accepts a closure argument with a receiver-typed row parameter.
    ///
    /// Queries the MethodTable for Table and DataTable first; falls back to
    /// the hardcoded heuristic for user-defined types or methods not yet in the table.
    fn is_datatable_closure_method(&self, method: &str) -> bool {
        if self
            .method_table
            .takes_closure_with_receiver_param("Table", method)
            || self
                .method_table
                .takes_closure_with_receiver_param("DataTable", method)
        {
            return true;
        }
        // Fallback: hardcoded heuristic for methods not registered in the MethodTable
        // (e.g., user-defined types, aliases like group_by/index_by)
        Self::is_datatable_closure_method_heuristic(method)
    }

    /// Hardcoded fallback for closure-method detection.
    fn is_datatable_closure_method_heuristic(method: &str) -> bool {
        matches!(
            method,
            "filter"
                | "forEach"
                | "map"
                | "find"
                | "some"
                | "every"
                | "groupBy"
                | "group_by"
                | "orderBy"
                | "index_by"
                | "indexBy"
                | "sum"
                | "mean"
                | "min"
                | "max"
                | "simulate"
        )
    }

    /// Check if a method preserves the Table<T> type (output is same Table<T> as input).
    ///
    /// Queries the MethodTable for Table, DataTable, and Array first; falls back to
    /// the hardcoded heuristic for user-defined types or methods not yet in the table.
    fn is_type_preserving_table_method(&self, method: &str) -> bool {
        if self.method_table.is_self_returning("Table", method)
            || self.method_table.is_self_returning("DataTable", method)
        {
            return true;
        }
        // Fallback: hardcoded heuristic for methods not registered in the MethodTable
        // (e.g., user-defined types, aliases like "where", "slice", "reverse", "concat")
        Self::is_type_preserving_table_method_heuristic(method)
    }

    /// Hardcoded fallback for type-preserving method detection.
    fn is_type_preserving_table_method_heuristic(method: &str) -> bool {
        matches!(
            method,
            "filter"
                | "where"
                | "head"
                | "tail"
                | "slice"
                | "reverse"
                | "concat"
                | "orderBy"
                | "sort"
        )
    }

    /// Returns true when the receiver is a module namespace object.
    ///
    /// Module receivers must dispatch as function value calls:
    /// `module.fn(args)` lowers to `CallValue` on the exported function value.
    fn is_module_namespace_receiver(&self, receiver: &Expr) -> bool {
        matches!(
            receiver,
            Expr::Identifier(name, _)
                if (name == "__comptime__" && self.allow_internal_comptime_namespace)
                    || self.module_namespace_bindings.contains(name)
        )
    }

    /// Compile a method call expression
    pub(super) fn compile_expr_method_call(
        &mut self,
        receiver: &Expr,
        method: &str,
        args: &[Expr],
    ) -> Result<()> {
        // Chained function calls: `f(a)(b)` is parsed as MethodCall with method "__call__".
        // Compile as: evaluate receiver (which produces a callable), compile args, CallValue.
        if method == "__call__" {
            self.compile_expr(receiver)?;
            let writebacks = self.compile_call_args(args, None)?;
            let arg_count = self
                .program
                .add_constant(Constant::Number(args.len() as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));
            self.emit(Instruction::simple(OpCode::CallValue));
            if !writebacks.is_empty() {
                let result_local = self.declare_temp_local("__chained_call_result_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));
                for (shadow_local, binding_idx) in writebacks {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(shadow_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
            }
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.last_expr_numeric_type = None;
            return Ok(());
        }

        // In-place mutation: arr.push(val) → ArrayPushLocal + LoadLocal
        // This is the primary push path for method calls inside function bodies,
        // loops, and blocks (which are compiled as expressions, not statements).
        if method == "push" && args.len() == 1 {
            if let Expr::Identifier(recv_name, _) = receiver {
                let source_loc = self.span_to_source_location(receiver.span());
                if let Some(local_idx) = self.resolve_local(recv_name) {
                    if !self.ref_locals.contains(&local_idx) {
                        self.check_named_binding_write_allowed(
                            recv_name,
                            Some(source_loc.clone()),
                        )?;
                    }
                    self.compile_expr(&args[0])?;
                    let pushed_numeric = self.last_expr_numeric_type;
                    self.emit(Instruction::new(
                        OpCode::ArrayPushLocal,
                        Some(Operand::Local(local_idx)),
                    ));
                    if let Some(numeric_type) = pushed_numeric {
                        self.mark_slot_as_numeric_array(local_idx, true, numeric_type);
                    }
                    // Push the mutated array as expression result
                    if self.ref_locals.contains(&local_idx)
                        || self.reference_value_locals.contains(&local_idx)
                    {
                        self.emit(Instruction::new(
                            OpCode::DerefLoad,
                            Some(Operand::Local(local_idx)),
                        ));
                    } else {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                    }
                    return Ok(());
                } else if !self
                    .mutable_closure_captures
                    .contains_key(recv_name.as_str())
                {
                    self.check_named_binding_write_allowed(recv_name, Some(source_loc))?;
                    let binding_idx = self.get_or_create_module_binding(recv_name);
                    self.compile_expr(&args[0])?;
                    self.emit(Instruction::new(
                        OpCode::ArrayPushLocal,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                    // Push the mutated array as expression result
                    self.emit(Instruction::new(
                        OpCode::LoadModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                    return Ok(());
                }
            }
        }

        // Universal type query: `expr.type()`.
        // Use static type constants when fully resolved; otherwise fall back to
        // runtime `TypeOf` so generic parameters resolve to concrete call-site types.
        if method == "type" {
            if !args.is_empty() {
                return Err(ShapeError::SemanticError {
                    message: "type() does not take any arguments".to_string(),
                    location: Some(self.span_to_source_location(receiver.span())),
                });
            }

            let is_type_symbol = self.expr_is_type_symbol(receiver);

            match self.static_type_annotation_for_expr(receiver) {
                Ok(type_ann) if !self.should_runtime_type_query(&type_ann) => {
                    // Preserve receiver side effects for expression receivers.
                    // For type symbols (e.g. Point.type()), skip value codegen.
                    if !is_type_symbol {
                        self.compile_expr(receiver)?;
                        self.emit(Instruction::simple(OpCode::Pop));
                    }

                    let idx = self
                        .program
                        .add_constant(Constant::TypeAnnotation(type_ann));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(idx)),
                    ));
                }
                Ok(_) => {
                    self.compile_expr(receiver)?;
                    self.emit(Instruction::new(
                        OpCode::BuiltinCall,
                        Some(Operand::Builtin(BuiltinFunction::TypeOf)),
                    ));
                }
                Err(err) => {
                    if is_type_symbol {
                        return Err(err);
                    }
                    self.compile_expr(receiver)?;
                    self.emit(Instruction::new(
                        OpCode::BuiltinCall,
                        Some(Operand::Builtin(BuiltinFunction::TypeOf)),
                    ));
                }
            }

            self.last_expr_schema = None;
            self.last_expr_numeric_type = None;
            self.last_expr_type_info = None;
            return Ok(());
        }

        // Universal formatting conversion: `expr.to_string()`.
        // Lower directly to FormatValueWithMeta so it shares exactly the same
        // rendering path as interpolation/print.
        if method == "to_string" || method == "toString" {
            if !args.is_empty() {
                return Err(ShapeError::SemanticError {
                    message: "to_string() does not take any arguments".to_string(),
                    location: Some(self.span_to_source_location(receiver.span())),
                });
            }

            self.compile_expr(receiver)?;

            let count = self.program.add_constant(Constant::Number(1.0));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(count)),
            ));
            self.emit(Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(BuiltinFunction::FormatValueWithMeta)),
            ));
            self.last_expr_schema = None;
            self.last_expr_numeric_type = None;
            self.last_expr_type_info = None;
            return Ok(());
        }

        // Removed legacy CSV namespace entrypoint.
        // Keep this specific to unresolved/module namespace access so local
        // variables named `csv` can still expose their own `load` method.
        if method == "load"
            && let Expr::Identifier(namespace_name, namespace_span) = receiver
            && namespace_name == "csv"
            && self.resolve_local(namespace_name).is_none()
            && !self.mutable_closure_captures.contains_key(namespace_name)
        {
            return Err(ShapeError::SemanticError {
                message: "csv.load(...) has been removed. Use a module-scoped data source API from a configured extension module."
                    .to_string(),
                location: Some(self.span_to_source_location(*namespace_span)),
            });
        }

        // Namespace calls (`module.fn(...)`) are function-style dispatch, not methods.
        if self.is_module_namespace_receiver(receiver) {
            return self.compile_module_namespace_call(receiver, method, args);
        }

        // DateTime static constructor methods: DateTime.now(), DateTime.utc(),
        // DateTime.parse(str), DateTime.from_epoch(ms)
        if let Expr::Identifier(name, _) = receiver {
            if name == "DateTime" || name == "Content" {
                let builtin = match (name.as_str(), method) {
                    ("DateTime", "now") => Some(BuiltinFunction::DateTimeNow),
                    ("DateTime", "utc") => Some(BuiltinFunction::DateTimeUtc),
                    ("DateTime", "parse") => Some(BuiltinFunction::DateTimeParse),
                    ("DateTime", "from_epoch") => Some(BuiltinFunction::DateTimeFromEpoch),
                    ("DateTime", "from_parts") => Some(BuiltinFunction::DateTimeFromParts),
                    ("DateTime", "from_unix_secs") => Some(BuiltinFunction::DateTimeFromUnixSecs),
                    ("Content", "chart") => Some(BuiltinFunction::ContentChart),
                    ("Content", "text") => Some(BuiltinFunction::ContentTextCtor),
                    ("Content", "table") => Some(BuiltinFunction::ContentTableCtor),
                    ("Content", "code") => Some(BuiltinFunction::ContentCodeCtor),
                    ("Content", "kv") => Some(BuiltinFunction::ContentKvCtor),
                    ("Content", "fragment") => Some(BuiltinFunction::ContentFragmentCtor),
                    _ => None,
                };
                if let Some(bf) = builtin {
                    // Compile arguments (if any) onto the stack
                    for arg in args {
                        self.compile_expr_as_value_or_placeholder(arg)?;
                    }
                    let count = self
                        .program
                        .add_constant(Constant::Number(args.len() as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(count)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::BuiltinCall,
                        Some(Operand::Builtin(bf)),
                    ));
                    self.last_expr_schema = None;
                    self.last_expr_numeric_type = None;
                    self.last_expr_type_info = None;
                    return Ok(());
                }
            }
        }

        // Comptime mini-programs may include scoped helper functions (`m::f`) without
        // materializing a runtime module object for `m`. Prefer direct scoped dispatch.
        if let Expr::Identifier(namespace, _) = receiver {
            let scoped_name = format!("{}::{}", namespace, method);
            if self.find_function(&scoped_name).is_some() {
                return self.compile_expr_function_call(&scoped_name, args, receiver.span());
            }
        }

        // Compile-time enforcement: resample/between require an Indexed table
        if method == "resample" || method == "between" {
            if let Expr::Identifier(name, span) = receiver {
                let is_indexed = self
                    .resolve_local(name)
                    .and_then(|idx| self.type_tracker.get_local_type(idx))
                    .map(|info| info.is_indexed())
                    .unwrap_or(false);
                let is_table = self
                    .resolve_local(name)
                    .and_then(|idx| self.type_tracker.get_local_type(idx))
                    .map(|info| info.is_datatable())
                    .unwrap_or(false);
                if is_table && !is_indexed {
                    return Err(ShapeError::RuntimeError {
                        message: format!(
                            "{}() requires an indexed table. Use .indexBy(row => row.column) first",
                            method
                        ),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
            }
        }

        // Compile receiver (the object/series being called)
        self.compile_expr(receiver)?;
        let receiver_schema = self.last_expr_schema;
        let receiver_type_info = self.last_expr_type_info.clone();
        // Capture receiver's numeric type for extend method return type propagation.
        let receiver_numeric_type = self.last_expr_numeric_type;
        // Capture receiver's extend type before args compilation overwrites compiler state.
        let receiver_extend_type =
            self.resolve_receiver_extend_type(receiver, &receiver_type_info, receiver_schema);

        // Resolve closure-row schema from the receiver contract.
        // `receiver` was compiled immediately above and may carry Table<T> metadata.
        if self.is_datatable_closure_method(method) {
            if let Some(ref info) = receiver_type_info {
                if let Some((schema_id, type_name)) = Self::table_schema_from_type_info(info) {
                    self.closure_row_schema = Some((schema_id, type_name));
                }
            } else if let Some(schema_id) = receiver_schema {
                if let Some((schema_id, type_name)) =
                    self.extract_table_schema_from_callable_field(schema_id, method)
                {
                    self.closure_row_schema = Some((schema_id, type_name));
                }
            }
        }

        // Save the receiver's Table<T> schema BEFORE compiling args.
        // Closure compilation resets expression metadata, so we must save it here.
        let receiver_table_schema = receiver_type_info
            .as_ref()
            .and_then(Self::table_schema_from_type_info);

        // Typed-object callable field dispatch:
        // `obj.field(args...)` where `field` is a typed property that stores a closure/function.
        // This is required for generated connection objects like `conn.candles()`.
        // Only dispatch this way when the field type could actually hold a callable
        // (Any, Object, Array). Primitive field types (int, number, bool, etc.) are
        // never callable, so `t.value()` with `value: int` must fall through to
        // the CallMethod path for trait method dispatch.
        if let Some(schema_id) = receiver_schema
            && let Some(schema) = self.type_tracker.schema_registry().get_by_id(schema_id)
            && let Some(field) = schema.get_field(method)
            && field.field_type.is_potentially_callable()
        {
            if schema_id > u16::MAX as u32 || field.offset > u16::MAX as usize {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "typed-field metadata exceeds limits for method-style field call '{}'",
                        method
                    ),
                    location: Some(self.span_to_source_location(receiver.span())),
                });
            }

            let operand = Operand::TypedField {
                type_id: schema_id as u16,
                field_idx: field.index as u16,
                field_type_tag: field_type_to_tag(&field.field_type),
            };
            self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));

            for arg in args {
                self.compile_expr_as_value_or_placeholder(arg)?;
            }

            let arg_count = self
                .program
                .add_constant(Constant::Number(args.len() as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));
            self.emit(Instruction::simple(OpCode::CallValue));

            self.last_expr_type_info = self
                .extract_table_schema_from_callable_field(schema_id, method)
                .map(|(sid, type_name)| VariableTypeInfo::datatable(sid, type_name));
            self.last_expr_schema = self
                .last_expr_type_info
                .as_ref()
                .and_then(Self::value_schema_from_type_info);
            self.last_expr_numeric_type = None;
            self.closure_row_schema = None;
            return Ok(());
        }

        // Compile arguments (closure_row_schema is consumed during closure compilation)
        for arg in args {
            self.compile_expr_as_value_or_placeholder(arg)?;
        }

        // Clear closure_row_schema after compiling args (in case it wasn't consumed)
        self.closure_row_schema = None;

        // UFCS: If a user-defined function exists with this name, prefer it over built-in methods.
        // This allows `extend` blocks to override built-in methods for specific types.
        // Rewrite `receiver.method(args)` → `method(receiver, args)`.
        //
        // Check bare function name first (user-defined free functions), then
        // extend-method qualified name "Type.method" using the captured receiver type.
        // For numeric types, also check parent type: Int → Number (Int is a subtype of
        // Number for method dispatch, so `extend Number` methods apply to Int values).
        let extend_func_idx = receiver_extend_type.as_deref().and_then(|type_name| {
            let qualified = format!("{}.{}", type_name, method);
            self.find_function(&qualified).or_else(|| {
                // Try parent type for subtypes (Int → Number)
                let parent = match type_name {
                    "Int" => Some("Number"),
                    _ => None,
                };
                parent.and_then(|p| {
                    let parent_qualified = format!("{}.{}", p, method);
                    self.find_function(&parent_qualified)
                })
            })
        });
        if let Some(func_idx) = extend_func_idx.or_else(|| self.find_function(method)) {
            // UFCS rewrite: receiver already compiled (on stack), args already compiled.
            // Stack is: [receiver, arg1, arg2, ...] — receiver is first, which is what we want.
            // Pad missing default args with Unit sentinels (same as regular call path).
            let func_name = self.program.functions[func_idx].name.clone();
            let total_arity = self.program.functions[func_idx].arity as usize;
            let effective_total_arity = self
                .function_arity_bounds
                .get(&func_name)
                .map(|(_, eff)| *eff)
                .unwrap_or(total_arity);
            let actual_arity_with_self = args.len() + 1;
            for _ in actual_arity_with_self..effective_total_arity {
                self.emit_unit();
            }
            let call_arity = actual_arity_with_self.max(effective_total_arity);
            let arg_count = self
                .program
                .add_constant(Constant::Number(call_arity as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));
            self.emit(Instruction::new(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(func_idx as u16))),
            ));
            // Record callee as a blob dependency
            if let Some(ref mut blob) = self.current_blob_builder {
                blob.record_call(&func_name);
            }
            self.last_expr_schema = None;
            // Propagate return type for UFCS method calls.
            // For extend methods (resolved via qualified Type.method name),
            // propagate the receiver's numeric type for chaining support.
            // For bare-name user functions, use the static method table.
            let resolved_via_extend =
                extend_func_idx.is_some() && self.find_function(method).is_none();
            self.last_expr_numeric_type = if resolved_via_extend {
                receiver_numeric_type
            } else {
                method_return_numeric_type(method)
            };
            // UFCS to user function: type-preserving methods still propagate Table<T>
            if self.is_type_preserving_table_method(method) {
                self.last_expr_type_info = receiver_type_info;
            } else {
                self.last_expr_type_info = None;
            }
            return Ok(());
        }

        // BUG-TR2 fix: Check for trait impl methods BEFORE falling through to builtin dispatch.
        // When the receiver has a known type (e.g., TypedObject with type_name "MyType"),
        // check if a trait impl method "MyType::method" or extend method "MyType.method"
        // exists. If so, dispatch it via direct Call instead of letting the builtin
        // with the same name shadow it.
        {
            // Use receiver_extend_type (covers both TypedObjects and primitives).
            // For subtypes (Int → Number), also try parent type methods.
            let extend_type_names: Vec<&str> = match receiver_extend_type.as_deref() {
                Some("Int") => vec!["Int", "Number"],
                Some(t) => vec![t],
                None => vec![],
            };
            // Check impl methods (Type::method) and extend methods (Type.method)
            let scoped_func_idx = extend_type_names.iter().find_map(|type_name| {
                let scoped_name = format!("{}::{}", type_name, method);
                let extend_name = format!("{}.{}", type_name, method);
                self.find_function(&scoped_name)
                    .or_else(|| self.find_function(&extend_name))
            });
            // Also check trait_method_symbols for named impls
            let trait_func_idx = scoped_func_idx
                .is_none()
                .then(|| {
                    extend_type_names.iter().find_map(|type_name| {
                        self.program
                            .find_default_trait_impl_for_type_method(type_name, method)
                            .map(|s| s.to_string())
                            .and_then(|impl_func_name| self.find_function(&impl_func_name))
                    })
                })
                .flatten();

            if let Some(func_idx) = scoped_func_idx.or(trait_func_idx) {
                let func_name = self.program.functions[func_idx].name.clone();
                let total_arity = self.program.functions[func_idx].arity as usize;
                let effective_total_arity = self
                    .function_arity_bounds
                    .get(&func_name)
                    .map(|(_, eff)| *eff)
                    .unwrap_or(total_arity);
                let actual_arity_with_self = args.len() + 1;
                for _ in actual_arity_with_self..effective_total_arity {
                    self.emit_unit();
                }
                let call_arity = actual_arity_with_self.max(effective_total_arity);
                let arg_count = self
                    .program
                    .add_constant(Constant::Number(call_arity as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(arg_count)),
                ));
                self.emit(Instruction::new(
                    OpCode::Call,
                    Some(Operand::Function(shape_value::FunctionId(func_idx as u16))),
                ));
                if let Some(ref mut blob) = self.current_blob_builder {
                    blob.record_call(&func_name);
                }
                self.last_expr_schema = None;
                self.last_expr_numeric_type = method_return_numeric_type(method);
                if self.is_type_preserving_table_method(method) {
                    self.last_expr_type_info = receiver_type_info;
                } else {
                    self.last_expr_type_info = None;
                }
                return Ok(());
            }
        }

        // Also check built-in intrinsics for UFCS (skip if it's a known built-in method name)
        if !Self::is_known_builtin_method(method) {
            if let Some(builtin) = self.get_builtin_function(method) {
                // UFCS to builtin: receiver + args already on stack
                let arg_count = self
                    .program
                    .add_constant(Constant::Number((args.len() + 1) as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(arg_count)),
                ));
                self.emit(Instruction::new(
                    OpCode::BuiltinCall,
                    Some(Operand::Builtin(builtin)),
                ));
                self.last_expr_schema = None;
                // Propagate known return type for UFCS builtin method calls
                self.last_expr_numeric_type = method_return_numeric_type(method);
                if self.is_type_preserving_table_method(method) {
                    self.last_expr_type_info = receiver_type_info;
                } else {
                    self.last_expr_type_info = None;
                }
                return Ok(());
            }
        }

        // Standard method call dispatch (runtime via CallMethod opcode)
        // Resolve method name to a typed MethodId at compile time
        let method_id = shape_value::MethodId::from_name(method);
        let string_idx = self.program.add_string(method.to_string());
        self.emit(Instruction::new(
            OpCode::CallMethod,
            Some(Operand::TypedMethodCall {
                method_id: method_id.0,
                arg_count: args.len() as u16,
                string_id: string_idx,
            }),
        ));
        // Propagate known return type for standard method calls
        self.last_expr_schema = None;
        self.last_expr_numeric_type = method_return_numeric_type(method);

        // Propagate Table<T> type through type-preserving methods.
        // After filter/head/tail/etc., the result is still Table<T>.
        if self.is_type_preserving_table_method(method) {
            self.last_expr_type_info = receiver_type_info.clone();
        } else {
            self.last_expr_type_info = None;
        }

        // Track indexBy result: extract field name from closure arg at compile time
        if (method == "indexBy" || method == "index_by") && receiver_table_schema.is_some() {
            if let Some((schema_id, ref type_name)) = receiver_table_schema {
                let index_col = args.first().and_then(Self::extract_closure_field_name);
                if let Some(col_name) = index_col {
                    self.last_expr_type_info = Some(VariableTypeInfo::indexed(
                        schema_id,
                        type_name.clone(),
                        col_name,
                    ));
                }
            }
        }

        Ok(())
    }

    fn compile_module_namespace_call(
        &mut self,
        receiver: &Expr,
        method: &str,
        args: &[Expr],
    ) -> Result<()> {
        let Expr::Identifier(namespace_name, namespace_span) = receiver else {
            return Err(ShapeError::SemanticError {
                message: "module namespace call must use an identifier receiver".to_string(),
                location: Some(self.span_to_source_location(receiver.span())),
            });
        };

        // Detect json.parse(text, TypeName) → rewrite to json.__parse_typed(text, schema_id).
        // When the second arg is a type identifier with a registered schema, we compile
        // a typed deserialization call that uses @alias annotations and field types.
        if namespace_name == "json" && method == "parse" && args.len() == 2 {
            if let Expr::Identifier(type_name, _) = &args[1] {
                if let Some(target_schema) = self.type_tracker.schema_registry().get(type_name) {
                    let target_schema_id = target_schema.id;
                    // Rewrite: compile as json.__parse_typed(text, schema_id)
                    let schema_id_expr =
                        Expr::Literal(Literal::Number(target_schema_id as f64), args[1].span());
                    let rewritten_args = vec![args[0].clone(), schema_id_expr];
                    return self.compile_module_namespace_call(
                        receiver,
                        "__parse_typed",
                        &rewritten_args,
                    );
                }
            }
        }

        // Shape-source module exports (non-native) compile as regular functions.
        // Route namespace calls to direct function dispatch so const-template
        // specialization/comptime handlers run in the same compiler context.
        if !self.is_native_module_export(namespace_name, method)
            && self.program.functions.iter().any(|f| f.name == method)
        {
            return self.compile_expr_function_call(method, args, receiver.span());
        }

        if self.is_native_module_export(namespace_name, method)
            && !self.is_native_module_export_available(namespace_name, method)
        {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "module export '{}.{}' is only available in comptime contexts",
                    namespace_name, method
                ),
                location: Some(self.span_to_source_location(*namespace_span)),
            });
        }

        self.compile_expr(receiver)?;
        let schema_id = self
            .last_expr_schema
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!(
                    "module namespace '{}' is not typed. Missing module schema for property '{}'",
                    namespace_name, method
                ),
                location: Some(self.span_to_source_location(*namespace_span)),
            })?;

        let Some(schema) = self.type_tracker.schema_registry().get_by_id(schema_id) else {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "module namespace '{}' schema id {} is not registered",
                    namespace_name, schema_id
                ),
                location: Some(self.span_to_source_location(*namespace_span)),
            });
        };

        let Some(field) = schema.get_field(method) else {
            return Err(ShapeError::SemanticError {
                message: format!("module '{}' has no export '{}'", namespace_name, method),
                location: Some(self.span_to_source_location(*namespace_span)),
            });
        };

        if schema_id > u16::MAX as u32 || field.offset > u16::MAX as usize {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "module '{}' export metadata exceeds typed-field limits for '{}'",
                    namespace_name, method
                ),
                location: Some(self.span_to_source_location(*namespace_span)),
            });
        }
        let operand = Operand::TypedField {
            type_id: schema_id as u16,
            field_idx: field.index as u16,
            field_type_tag: field_type_to_tag(&field.field_type),
        };
        self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));

        for arg in args {
            self.compile_expr_as_value_or_placeholder(arg)?;
        }

        let arg_count = self
            .program
            .add_constant(Constant::Number(args.len() as f64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(arg_count)),
        ));
        self.emit(Instruction::simple(OpCode::CallValue));

        let namespace_call_expr = Expr::MethodCall {
            receiver: Box::new(receiver.clone()),
            method: method.to_string(),
            args: args.to_vec(),
            named_args: vec![],
            span: receiver.span(),
        };
        let inferred = self.infer_expr_type(&namespace_call_expr).ok();
        self.last_expr_type_info = inferred
            .as_ref()
            .and_then(|ty| self.type_info_from_inferred_type(ty));
        self.last_expr_schema = self
            .last_expr_type_info
            .as_ref()
            .and_then(Self::value_schema_from_type_info);
        self.last_expr_numeric_type = None;
        Ok(())
    }

    /// Extract the field name from a simple closure like `row => row.field`.
    /// Returns Some("field") if the closure is a single property access on the parameter.
    fn extract_closure_field_name(expr: &Expr) -> Option<String> {
        if let Expr::FunctionExpr { params, body, .. } = expr {
            if params.len() != 1 {
                return None;
            }
            let param_name = params[0].simple_name()?;

            // Check body: either [Return(Some(PropertyAccess))] or [Expression(PropertyAccess)]
            if body.len() != 1 {
                return None;
            }
            let inner = match &body[0] {
                shape_ast::ast::Statement::Return(Some(e), _) => e,
                shape_ast::ast::Statement::Expression(e, _) => e,
                _ => return None,
            };

            if let Expr::PropertyAccess {
                object, property, ..
            } = inner
            {
                if let Expr::Identifier(name, _) = object.as_ref() {
                    if name == param_name {
                        return Some(property.clone());
                    }
                }
            }
        }
        None
    }

    /// Compile print call with string interpolation expansion
    ///
    /// For strings with `{expr}`, expands at compile time:
    /// - Literal parts: pushed as string constants
    /// - Expression parts: parsed, compiled, converted to string
    /// - Parts are concatenated with Add
    fn compile_print_with_interpolation(&mut self, args: &[Expr]) -> Result<()> {
        let mut processed_args = 0;

        for arg in args {
            // Check if this is a string literal with interpolation
            if let Expr::Literal(Literal::String(s), _span) = arg {
                if has_interpolation(s) {
                    // Expand the interpolation
                    if let Err(err) =
                        self.compile_interpolated_string_expression(s, InterpolationMode::Braces)
                    {
                        if self.should_recover_compile_diagnostics() {
                            self.errors.push(err);
                            self.emit(Instruction::simple(OpCode::PushNull));
                        } else {
                            return Err(err);
                        }
                    }
                    processed_args += 1;
                    continue;
                }
            }

            // Normal argument - compile as-is
            self.compile_expr_as_value_or_placeholder(arg)?;
            processed_args += 1;
        }

        // Push arg count and call print
        let arg_count = self
            .program
            .add_constant(Constant::Number(processed_args as f64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(arg_count)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::Print)),
        ));

        self.last_expr_schema = None;
        self.last_expr_type_info = None;
        self.last_expr_numeric_type = None;

        Ok(())
    }

    /// Collect all available function names for suggestions
    fn collect_available_function_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        // User-defined functions
        for func in &self.program.functions {
            names.push(func.name.clone());
        }
        // Builtin function names (common ones only, skip intrinsics)
        let builtins = [
            "abs",
            "min",
            "max",
            "sqrt",
            "ln",
            "pow",
            "exp",
            "log",
            "floor",
            "ceil",
            "round",
            "sin",
            "cos",
            "tan",
            "stddev",
            "slice",
            "push",
            "pop",
            "first",
            "last",
            "zip",
            "map",
            "filter",
            "reduce",
            "forEach",
            "find",
            "findIndex",
            "some",
            "every",
            "print",
            "format",
            "len",
            "count",
            "range",
            "sum",
            "mean",
            "std",
            "variance",
        ];
        for name in builtins {
            names.push(name.to_string());
        }
        names
    }

    /// Check if a function name is a comptime-only builtin.
    /// These are only callable inside `comptime { }` blocks and are rejected
    /// during normal compilation with a helpful error message.
    fn is_comptime_only_builtin(name: &str) -> bool {
        shape_runtime::builtin_metadata::is_comptime_builtin_function(name)
    }
}
