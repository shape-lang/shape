//! Function and method call expression compilation

use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::compiler::monomorphization::type_resolution::{
    concrete_type_for_expr, extract_arg_concrete_types, resolve_call_site_type_args,
};
use crate::compiler::string_interpolation::has_interpolation;
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::{NumericType, VariableKind, VariableTypeInfo};
use shape_ast::ast::{BinaryOp, Expr, InterpolationMode, Literal, Span, Spanned, UnaryOp};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_system::suggestions::suggest_function;
use shape_runtime::type_system::{BuiltinTypes, Type};
use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;

use super::super::{BuiltinNameResolution, BytecodeCompiler, ModuleBuiltinFunction};

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
    pub(crate) fn hidden_native_module_binding_name(module_path: &str) -> String {
        format!("__imported_module__::{}", module_path)
    }

    fn ensure_hidden_native_module_binding(&mut self, module_path: &str) -> String {
        let binding_name = Self::hidden_native_module_binding_name(module_path);
        if !self.module_bindings.contains_key(&binding_name) {
            let binding_idx = self.get_or_create_module_binding(&binding_name);
            self.register_extension_module_schema(module_path);
            let module_schema_name = format!("__mod_{}", module_path);
            if self
                .type_tracker
                .schema_registry()
                .get(&module_schema_name)
                .is_some()
            {
                self.set_module_binding_type_info(binding_idx, &module_schema_name);
            }
        }
        binding_name
    }

    fn compile_module_builtin_function_call(
        &mut self,
        builtin_decl: &ModuleBuiltinFunction,
        args: &[Expr],
        span: Span,
    ) -> Result<()> {
        if !self.is_native_module_export(
            &builtin_decl.source_module_path,
            &builtin_decl.export_name,
        ) {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "builtin function '{}' has no runtime implementation in module '{}'",
                    builtin_decl.export_name, builtin_decl.source_module_path
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }
        let binding_name = self.ensure_hidden_native_module_binding(&builtin_decl.source_module_path);
        self.compile_module_namespace_call_on_binding(
            &binding_name,
            &builtin_decl.source_module_path,
            span,
            &builtin_decl.export_name,
            args,
        )
    }

    fn resolve_scoped_module_builtin_function(
        &self,
        name: &str,
    ) -> Option<ModuleBuiltinFunction> {
        if let Some(decl) = self.module_builtin_functions.get(name) {
            return Some(decl.clone());
        }

        for module_path in self.module_scope_stack.iter().rev() {
            let candidate = format!("{}::{}", module_path, name);
            if let Some(decl) = self.module_builtin_functions.get(&candidate) {
                return Some(decl.clone());
            }
        }
        None
    }

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
            shape_ast::ast::TypeAnnotation::Basic(name) => self
                .type_tracker
                .schema_registry()
                .get(name.as_str())
                .map(|schema| (schema.id, name.clone())),
            shape_ast::ast::TypeAnnotation::Reference(name) => self
                .type_tracker
                .schema_registry()
                .get(name.as_str())
                .map(|schema| (schema.id, name.to_string())),
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
            shape_ast::ast::TypeAnnotation::Basic(name) => self
                .type_tracker
                .schema_registry()
                .get(name.as_str())
                .map(|schema| VariableTypeInfo::known(schema.id, name.clone())),
            shape_ast::ast::TypeAnnotation::Reference(name) => self
                .type_tracker
                .schema_registry()
                .get(name.as_str())
                .map(|schema| VariableTypeInfo::known(schema.id, name.to_string())),
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
            .and_then(|registry| {
                registry
                    .iter()
                    .rev()
                    .find(|m| m.name == module_name)
            })
            .is_some_and(|module| module.has_export(export_name))
    }

    fn is_native_module_export_available(&self, module_name: &str, export_name: &str) -> bool {
        self.extension_registry
            .as_ref()
            .and_then(|registry| {
                registry
                    .iter()
                    .rev()
                    .find(|m| m.name == module_name)
            })
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
        // For module-scoped functions (e.g. myext::connect), temporarily push
        // the module path so annotation name resolution can find annotations
        // that were compiled within that module (e.g. myext::force_int).
        let module_prefix = name
            .rsplit_once("::")
            .map(|(prefix, _)| prefix.to_string());
        if let Some(ref prefix) = module_prefix {
            self.module_scope_stack.push(prefix.clone());
        }
        let has_comptime_handlers = template_def.annotations.iter().any(|ann| {
            self.lookup_compiled_annotation(ann)
                .map(|(_, compiled)| {
                    compiled.comptime_pre_handler.is_some()
                        || compiled.comptime_post_handler.is_some()
                })
                .unwrap_or(false)
        });
        if module_prefix.is_some() {
            self.module_scope_stack.pop();
        }
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

        // Push module scope for the specialization so annotation resolution
        // can find annotations defined in the original function's module.
        if let Some(ref prefix) = module_prefix {
            self.module_scope_stack.push(prefix.clone());
        }
        let compile_result = self.compile_function(&specialized_def);
        if module_prefix.is_some() {
            self.module_scope_stack.pop();
        }
        if let Err(err) = compile_result {
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
            let expected_param_modes = if let Some(local_idx) = self.resolve_local(name) {
                self.local_callable_pass_modes.get(&local_idx).cloned()
            } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
                self.module_bindings
                    .get(&scoped_name)
                    .and_then(|binding_idx| {
                        self.module_binding_callable_pass_modes
                            .get(binding_idx)
                            .cloned()
                    })
            } else {
                None
            };
            let return_reference_summary = self.function_return_reference_summary_for_name(name);
            // Use compile_expr_identifier to correctly load the callee value,
            // handling ref_locals (DerefLoad), mutable closure captures (LoadClosure), etc.
            self.compile_expr_identifier(name, span)?;

            let writebacks = self.compile_call_args(args, expected_param_modes.as_deref())?;
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
            if let Some(return_reference_summary) = return_reference_summary {
                self.set_last_expr_reference_result(return_reference_summary.mode, true);
            } else {
                self.clear_last_expr_reference_result();
            }
            return Ok(());
        }

        // Check for user-defined functions (after locals — function parameters take priority)
        if let Some(func_idx) = self.find_function(name) {
            let resolved_name = self.program.functions[func_idx].name.clone();

            // Check if this function was removed by a comptime annotation handler.
            if self.removed_functions.contains(&resolved_name)
                || self.removed_functions.contains(name)
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "function '{}' was removed by a comptime annotation handler and cannot be called",
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
            let return_reference_summary =
                self.function_return_reference_summary_for_name(&call_name);
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
                                if is_ref_param {
                                    let borrow_mode =
                                        if ref_mutates.get(param_idx).copied().unwrap_or(false) {
                                            crate::compiler::BorrowMode::Exclusive
                                        } else {
                                            crate::compiler::BorrowMode::Shared
                                        };
                                    self.compile_implicit_reference_arg(default_expr, borrow_mode)?;
                                }
                                if !is_ref_param {
                                    self.compile_expr(default_expr)?;
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
            if let Some(return_reference_summary) = return_reference_summary {
                self.set_last_expr_reference_result(return_reference_summary.mode, true);
            } else {
                self.clear_last_expr_reference_result();
            }
            return Ok(());
        }

        if let Some(builtin_decl) = self.resolve_scoped_module_builtin_function(name) {
            return self.compile_module_builtin_function_call(&builtin_decl, args, span);
        }

        // Builtins take precedence - they're optimized Rust implementations.
        // Phase 1 keeps the current surface behavior, but distinguishes
        // surface names from internal-only intrinsics for diagnostics.
        if let Some(resolution) = self.classify_builtin_function(name) {
            let builtin = match resolution {
                BuiltinNameResolution::Surface { builtin, .. } => builtin,
                BuiltinNameResolution::InternalOnly { builtin, .. }
                    if self.allow_internal_builtins =>
                {
                    builtin
                }
                BuiltinNameResolution::InternalOnly { .. } => {
                    return Err(ShapeError::SemanticError {
                        message: self.internal_intrinsic_error_message(name, resolution),
                        location: Some(self.span_to_source_location(span)),
                    });
                }
            };

            // Special handling for print with string interpolation
            if builtin == BuiltinFunction::Print {
                return self.compile_print_with_interpolation(args);
            }

            // v2 Phase 3.2: HashMap() typed-map fast path. When the call site's
            // surrounding context resolves K and V to a typed-map kind, lower
            // the constructor to a `NewTypedMap*` opcode instead of the
            // legacy `BuiltinCall(HashMapCtor)`. Falls through for any
            // unresolved K/V pair.
            if builtin == BuiltinFunction::HashMapCtor && args.is_empty() {
                use crate::compiler::v2_map_emission::infer_hashmap_kv_from_context;
                use crate::compiler::v2_typed_map_emission::should_use_typed_map;

                // Synthesize a fake call expression so we can query the
                // span-based side table. The call has no AST node here, so
                // we use a dummy expression with the call span — the only
                // shape `infer_hashmap_kv_from_context` actually queries.
                let dummy = Expr::Identifier(name.to_string(), span);
                if let Some((k, v)) = infer_hashmap_kv_from_context(self, &dummy) {
                    if let Some(kind) = should_use_typed_map(&k, &v) {
                        self.emit(Instruction::simple(kind.new_opcode()));
                        // Record the kv pair for the call expression's span so
                        // downstream method dispatch can use it without
                        // re-inference.
                        self.record_map_key_value_for_node(span, k, v);
                        // Propagate basic metadata so subsequent ops see a
                        // HashMap-shaped value.
                        self.last_expr_numeric_type = None;
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.clear_last_expr_reference_result();
                        return Ok(());
                    }
                }
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
            self.clear_last_expr_reference_result();
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

        // Named import from a native extension module (e.g. `from std::core::file use { read_text }`).
        // Native modules have no AST to inline, so the function won't be in program.functions.
        // Keep a private module binding so the imported symbol can dispatch without
        // implicitly creating a user-visible namespace.
        if let Some(imported) = self.imported_names.get(name).cloned() {
            if self.is_native_module_export(&imported.module_path, &imported.original_name) {
                let binding_name = self.ensure_hidden_native_module_binding(&imported.module_path);
                return self.compile_module_namespace_call_on_binding(
                    &binding_name,
                    &imported.module_path,
                    span,
                    &imported.original_name,
                    args,
                );
            }
        }

        // Build error message with suggestions
        let mut message = self.undefined_function_message(name);

        // Try import suggestion first
        if let Some(module_path) = self.suggest_import(name) {
            message = format!(
                "Unknown function '{}'. Did you mean to import it via '{}'\n\n  from {} use {{ {} }}\n\n{}",
                name,
                module_path,
                module_path,
                name,
                Self::function_scope_summary(),
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

    pub(super) fn is_module_namespace_name(&self, name: &str) -> bool {
        (name == "__comptime__" && self.allow_internal_comptime_namespace)
            || self.module_namespace_bindings.contains(name)
    }

    fn compile_type_namespace_builtin_call(
        &mut self,
        namespace: &str,
        function: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<bool> {
        let builtin = match (namespace, function) {
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

        let Some(builtin) = builtin else {
            return Ok(false);
        };

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
            Some(Operand::Builtin(builtin)),
        ));
        self.last_expr_schema = None;
        self.last_expr_numeric_type = None;
        self.last_expr_type_info = None;
        self.clear_last_expr_reference_result();
        let _ = span;
        Ok(true)
    }

    pub(super) fn compile_expr_qualified_function_call(
        &mut self,
        namespace: &str,
        function: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<()> {
        let scoped_name = format!("{}::{}", namespace, function);
        if let Some(builtin_decl) = self.module_builtin_functions.get(&scoped_name).cloned() {
            return self.compile_module_builtin_function_call(&builtin_decl, args, span);
        }
        if self.find_function(&scoped_name).is_some() {
            return self.compile_expr_function_call(&scoped_name, args, span);
        }

        if self.is_module_namespace_name(namespace) {
            return self.compile_module_namespace_call(namespace, span, function, args);
        }

        if self.compile_type_namespace_builtin_call(namespace, function, args, span)? {
            return Ok(());
        }

        if let Some(schema) = self.type_tracker.schema_registry().get(namespace)
            && let Some(enum_info) = schema.get_enum_info()
            && enum_info.variant_by_name(function).is_some()
        {
            return self.compile_expr_enum_constructor(
                namespace,
                function,
                &shape_ast::ast::EnumConstructorPayload::Tuple(args.to_vec()),
            );
        }

        Err(ShapeError::RuntimeError {
            message: format!(
                "Unknown qualified call '{}::{}'. Module namespace calls require an explicit `use`, and type-associated calls require the type to define that item.",
                namespace, function
            ),
            location: Some(self.span_to_source_location(span)),
        })
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
            let expected_param_modes = self.callable_pass_modes_from_expr(receiver);
            let return_reference_summary =
                self.callable_return_reference_summary_from_expr(receiver);
            self.compile_expr(receiver)?;
            let writebacks = self.compile_call_args(args, expected_param_modes.as_deref())?;
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
            if let Some(return_reference_summary) = return_reference_summary {
                self.set_last_expr_reference_result(return_reference_summary.mode, true);
            } else {
                self.clear_last_expr_reference_result();
            }
            return Ok(());
        }

        // In-place mutation: arr.push(val) → ArrayPushLocal + LoadLocal
        // This is the primary push path for method calls inside function bodies,
        // loops, and blocks (which are compiled as expressions, not statements).
        if method == "push" && args.len() == 1 {
            if let Expr::Identifier(recv_name, _) = receiver {
                // v2 Phase 3.1 (Agent 3): typed-array fast path for `arr.push(x)`.
                // Resolved BEFORE arg compilation since compile_expr may
                // overwrite tracker state. Falls through to legacy
                // `ArrayPushLocal` for non-typed arrays / unrecognised
                // element types.
                let typed_kind = self.resolve_receiver_typed_array_kind(receiver);
                let source_loc = self.span_to_source_location(receiver.span());
                if let Some(local_idx) = self.resolve_local(recv_name) {
                    if !self.ref_locals.contains(&local_idx) {
                        self.check_named_binding_write_allowed(
                            recv_name,
                            Some(source_loc.clone()),
                        )?;
                    }
                    if let Some(kind) = typed_kind {
                        // v2 typed array push: `TypedArrayPush*` pops
                        // (arr_ptr, value). Push the array, then the value,
                        // then the typed opcode.
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                        self.compile_expr(&args[0])?;
                        self.emit(Instruction::simple(kind.push_opcode()));
                        // Push the mutated array as expression result.
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
                        self.clear_last_expr_reference_result();
                        return Ok(());
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
                    self.clear_last_expr_reference_result();
                    return Ok(());
                } else if !self
                    .mutable_closure_captures
                    .contains_key(recv_name.as_str())
                {
                    self.check_named_binding_write_allowed(recv_name, Some(source_loc))?;
                    let binding_idx = self.get_or_create_module_binding(recv_name);
                    if let Some(kind) = typed_kind {
                        // v2 typed array push for module bindings.
                        self.emit(Instruction::new(
                            OpCode::LoadModuleBinding,
                            Some(Operand::ModuleBinding(binding_idx)),
                        ));
                        self.compile_expr(&args[0])?;
                        self.emit(Instruction::simple(kind.push_opcode()));
                        self.emit(Instruction::new(
                            OpCode::LoadModuleBinding,
                            Some(Operand::ModuleBinding(binding_idx)),
                        ));
                        self.clear_last_expr_reference_result();
                        return Ok(());
                    }
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
                    self.clear_last_expr_reference_result();
                    return Ok(());
                }
            }
        }

        // v2 Phase 3.2: HashMap typed-map fast path for `m.set/.get/.has/.delete`.
        //
        // Resolved BEFORE compiling the receiver because the typed opcodes
        // expect (map_ptr, key[, value]) on the stack with raw scalars where
        // appropriate. Falls through to the legacy CallMethod path when the
        // receiver isn't tracked as a typed map or when the method isn't one
        // of the four typed-map methods.
        if matches!(method, "set" | "get" | "has" | "delete")
            && self.is_typed_map_receiver(receiver)
        {
            if let Some(()) = self.try_compile_typed_map_method(receiver, method, args)? {
                return Ok(());
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
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        // Universal formatting conversion: `expr.to_string()`.
        // Lower directly to FormatValueWithMeta so it shares exactly the same
        // rendering path as interpolation/print.
        //
        // HOWEVER: if the receiver's type has a user-defined `to_string` method
        // (via an extend block or impl), we must NOT short-circuit here — the
        // user method should shadow the builtin.  We check this by looking for
        // any compiled function whose name ends in `.to_string`, `.toString`,
        // `::to_string`, or `::toString`.
        if (method == "to_string" || method == "toString")
            && !self.has_any_user_defined_method(method)
        {
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
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        if let Expr::Identifier(namespace_name, namespace_span) = receiver {
            if self.is_module_namespace_name(namespace_name)
                && self.resolve_local(namespace_name).is_none()
                && !self.mutable_closure_captures.contains_key(namespace_name.as_str())
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Module namespace calls must use `::`. Replace `{}.{}` with `{}::{}(...)`.",
                        namespace_name, method, namespace_name, method
                    ),
                    location: Some(self.span_to_source_location(*namespace_span)),
                });
            }

            // Removed legacy CSV namespace entrypoint.
            // Keep this specific to unresolved namespace-like access so local
            // variables named `csv` can still expose their own `load` method.
            if method == "load"
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

            if self.compile_type_namespace_builtin_call(namespace_name, method, args, *namespace_span)?
            {
                return Ok(());
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
            self.clear_last_expr_reference_result();
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
        if let Some(func_idx) = extend_func_idx
            .or_else(|| self.find_function(method))
            .filter(|&idx| self.current_function != Some(idx))
        {
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

            // --- Monomorphization: specialize generic extend methods ---
            //
            // When the resolved function has type parameters (e.g. `Vec<T>.indexOf`
            // where T is generic), try to monomorphize it for the receiver's
            // concrete element type. This produces a specialized function that
            // the v2 pipeline can emit typed opcodes for.
            //
            // Falls back to the generic function index on any failure.
            let call_func_idx = self
                .try_monomorphize_method_call(&func_name, receiver, args)
                .unwrap_or(func_idx);

            let arg_count = self
                .program
                .add_constant(Constant::Number(call_arity as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));

            let call_func_name = self.program.functions[call_func_idx].name.clone();
            self.emit(Instruction::new(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(
                    call_func_idx as u16,
                ))),
            ));
            // Record callee as a blob dependency
            if let Some(ref mut blob) = self.current_blob_builder {
                blob.record_call(&call_func_name);
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
            self.clear_last_expr_reference_result();
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

            if let Some(func_idx) = scoped_func_idx
                .or(trait_func_idx)
                .filter(|&idx| self.current_function != Some(idx))
            {
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

                // --- Monomorphization: specialize generic impl/trait methods ---
                //
                // When an impl method has synthesized type parameters (e.g.
                // `Array::findIndex` with T from the receiver's element type),
                // try to monomorphize it for the receiver's concrete type.
                // Falls back to the generic function index on any failure.
                let call_func_idx = self
                    .try_monomorphize_method_call(&func_name, receiver, args)
                    .unwrap_or(func_idx);

                let arg_count = self
                    .program
                    .add_constant(Constant::Number(call_arity as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(arg_count)),
                ));

                let call_func_name = self.program.functions[call_func_idx].name.clone();
                self.emit(Instruction::new(
                    OpCode::Call,
                    Some(Operand::Function(shape_value::FunctionId(
                        call_func_idx as u16,
                    ))),
                ));
                if let Some(ref mut blob) = self.current_blob_builder {
                    blob.record_call(&call_func_name);
                }
                self.last_expr_schema = None;
                self.last_expr_numeric_type = method_return_numeric_type(method);
                if self.is_type_preserving_table_method(method) {
                    self.last_expr_type_info = receiver_type_info;
                } else {
                    self.last_expr_type_info = None;
                }
                self.clear_last_expr_reference_result();
                return Ok(());
            }
        }

        // Also check built-in intrinsics for UFCS (skip if it's a known built-in method name)
        if !Self::is_known_builtin_method(method) {
            if let Some(resolution) = self.classify_builtin_function(method) {
                let builtin = match resolution {
                    BuiltinNameResolution::Surface { builtin, .. } => builtin,
                    BuiltinNameResolution::InternalOnly { builtin, .. }
                        if self.allow_internal_builtins =>
                    {
                        builtin
                    }
                    BuiltinNameResolution::InternalOnly { .. } => {
                        return Err(ShapeError::SemanticError {
                            message: self.internal_intrinsic_error_message(method, resolution),
                            location: Some(self.span_to_source_location(receiver.span())),
                        });
                    }
                };

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
                self.clear_last_expr_reference_result();
                return Ok(());
            }
        }

        // Standard method call dispatch (runtime via CallMethod opcode)
        // Resolve method name to a typed MethodId at compile time
        let method_id = shape_value::MethodId::from_name(method);
        let string_idx = self.program.add_string(method.to_string());

        // Resolve receiver ConcreteType tag for type-tagged dispatch
        let rtt = Self::resolve_type_tag(receiver_numeric_type, &receiver_type_info);

        self.emit(Instruction::new(
            OpCode::CallMethod,
            Some(Operand::TypedMethodCall {
                method_id: method_id.0,
                arg_count: args.len() as u16,
                string_id: string_idx,
                receiver_type_tag: rtt,
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

        self.clear_last_expr_reference_result();
        Ok(())
    }

    fn compile_module_namespace_call(
        &mut self,
        namespace_name: &str,
        namespace_span: Span,
        method: &str,
        args: &[Expr],
    ) -> Result<()> {
        self.compile_module_namespace_call_on_binding(
            namespace_name,
            namespace_name,
            namespace_span,
            method,
            args,
        )
    }

    fn compile_module_namespace_call_on_binding(
        &mut self,
        binding_name: &str,
        namespace_name: &str,
        namespace_span: Span,
        method: &str,
        args: &[Expr],
    ) -> Result<()> {
        // Detect json.parse(text, TypeName) → rewrite to json.__parse_typed(text, schema_id).
        // When the second arg is a type identifier with a registered schema, we compile
        // a typed deserialization call that uses @alias annotations and field types.
        // Resolve canonical module path: namespace_name may be a local alias ("json")
        // or already canonical ("std::core::json").
        let canonical_module = self
            .resolve_canonical_module_path(namespace_name)
            .unwrap_or_else(|| namespace_name.to_string());
        if canonical_module == "std::core::json" && method == "parse" && args.len() == 2 {
            if let Expr::Identifier(type_name, _) = &args[1] {
                if let Some(target_schema) = self.type_tracker.schema_registry().get(type_name) {
                    let target_schema_id = target_schema.id;
                    // Rewrite: compile as json.__parse_typed(text, schema_id)
                    let schema_id_expr =
                        Expr::Literal(Literal::Number(target_schema_id as f64), args[1].span());
                    let rewritten_args = vec![args[0].clone(), schema_id_expr];
                    return self.compile_module_namespace_call_on_binding(
                        binding_name,
                        namespace_name,
                        namespace_span,
                        "__parse_typed",
                        &rewritten_args,
                    );
                }
            }
        }

        // Shape-source module exports (non-native) compile as regular functions.
        // Route namespace calls to direct function dispatch so const-template
        // specialization/comptime handlers run in the same compiler context.
        let scoped_name = format!("{}::{}", namespace_name, method);
        if !self.is_native_module_export(namespace_name, method)
            && self.find_function(&scoped_name).is_some()
        {
            return self.compile_expr_function_call(&scoped_name, args, namespace_span);
        }

        if self.is_native_module_export(namespace_name, method)
            && !self.is_native_module_export_available(namespace_name, method)
        {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "module export '{}::{}' is only available in comptime contexts",
                    namespace_name, method
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        }

        // For native module exports, use a hidden binding so that the native
        // module object is not clobbered when a Shape artifact module with the
        // same name is compiled (the module decl overwrites the regular binding).
        let effective_binding_name = if self.is_native_module_export(namespace_name, method) {
            self.ensure_hidden_native_module_binding(namespace_name)
        } else {
            binding_name.to_string()
        };

        let binding_idx =
            *self
                .module_bindings
                .get(&effective_binding_name)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "module namespace '{}' is not bound in the current scope",
                        namespace_name
                    ),
                    location: Some(self.span_to_source_location(namespace_span)),
                })?;
        self.emit(Instruction::new(
            OpCode::LoadModuleBinding,
            Some(Operand::ModuleBinding(binding_idx)),
        ));
        self.last_expr_type_info = self.type_tracker.get_binding_type(binding_idx).cloned();
        self.last_expr_schema = self
            .last_expr_type_info
            .as_ref()
            .and_then(Self::value_schema_from_type_info);

        let schema_id = self.last_expr_schema.ok_or_else(|| ShapeError::SemanticError {
            message: format!(
                "module namespace '{}' is not typed. Missing module schema for export '{}'",
                namespace_name, method
            ),
            location: Some(self.span_to_source_location(namespace_span)),
        })?;

        let Some(schema) = self.type_tracker.schema_registry().get_by_id(schema_id) else {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "module namespace '{}' schema id {} is not registered",
                    namespace_name, schema_id
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        };

        let Some(field) = schema.get_field(method) else {
            return Err(ShapeError::SemanticError {
                message: format!("module '{}' has no export '{}'", namespace_name, method),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        };

        if schema_id > u16::MAX as u32 || field.offset > u16::MAX as usize {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "module '{}' export metadata exceeds typed-field limits for '{}'",
                    namespace_name, method
                ),
                location: Some(self.span_to_source_location(namespace_span)),
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

        let namespace_call_expr = Expr::QualifiedFunctionCall {
            namespace: namespace_name.to_string(),
            function: method.to_string(),
            args: args.to_vec(),
            named_args: vec![],
            span: namespace_span,
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

    /// v2 Phase 3.2: emit a typed-map opcode sequence for `m.set(k, v)`,
    /// `m.get(k)`, `m.has(k)`, or `m.delete(k)` when the receiver `m` is
    /// tracked as a v2 typed map. Returns `Ok(Some(()))` on success and
    /// `Ok(None)` when the receiver isn't a typed map (caller should fall
    /// through to the legacy `CallMethod` path).
    pub(super) fn try_compile_typed_map_method(
        &mut self,
        receiver: &Expr,
        method: &str,
        args: &[Expr],
    ) -> Result<Option<()>> {
        let kind = match self.resolve_receiver_typed_map_kind(receiver) {
            Some(k) => k,
            None => return Ok(None),
        };
        // Compile receiver to put map_ptr on the stack.
        self.compile_expr(receiver)?;

        match method {
            "set" => {
                if args.len() != 2 {
                    // Wrong arity — fall back to the legacy path.
                    return Ok(None);
                }
                self.compile_expr_as_value_or_placeholder(&args[0])?;
                self.compile_expr_as_value_or_placeholder(&args[1])?;
                self.emit(Instruction::simple(kind.set_opcode()));
                // set() returns the map itself for fluent chaining; mirror
                // that by re-loading the receiver.
                self.compile_expr(receiver)?;
            }
            "get" => {
                if args.len() != 1 {
                    return Ok(None);
                }
                self.compile_expr_as_value_or_placeholder(&args[0])?;
                self.emit(Instruction::simple(kind.get_opcode()));
            }
            "has" => {
                if args.len() != 1 {
                    return Ok(None);
                }
                self.compile_expr_as_value_or_placeholder(&args[0])?;
                self.emit(Instruction::simple(kind.has_opcode()));
            }
            "delete" => {
                if args.len() != 1 {
                    return Ok(None);
                }
                self.compile_expr_as_value_or_placeholder(&args[0])?;
                self.emit(Instruction::simple(kind.delete_opcode()));
                // delete() returns the map itself for chaining.
                self.compile_expr(receiver)?;
            }
            _ => return Ok(None),
        }
        self.last_expr_schema = None;
        self.last_expr_numeric_type = None;
        self.last_expr_type_info = None;
        self.clear_last_expr_reference_result();
        Ok(Some(()))
    }

    /// Attempt to monomorphize a generic extend method for the receiver's
    /// concrete type. Returns `Some(specialized_func_idx)` on success, or
    /// `None` if monomorphization is not applicable or fails.
    ///
    /// This is the bridge between generic extend methods (e.g. `Vec<T>.indexOf`)
    /// and the monomorphization cache. When the receiver has a concretely known
    /// type (e.g. `Array<int>`), the function's type parameters are resolved
    /// and a specialized version is compiled/cached.
    fn try_monomorphize_method_call(
        &mut self,
        func_name: &str,
        receiver: &Expr,
        args: &[Expr],
    ) -> Option<usize> {
        // 1. Check if the function has type parameters.
        let type_params: Vec<String> = {
            let def = self.function_defs.get(func_name)?;
            let tps = def.type_params.as_ref()?;
            if tps.is_empty() {
                return None;
            }
            tps.iter().map(|tp| tp.name.clone()).collect()
        };

        // 2. Build combined arg_types: [receiver_concrete_type, arg1_ct, ...].
        //    The function's first param is `self` (the receiver), followed by
        //    the explicit method arguments.
        let receiver_ct = concrete_type_for_expr(self, receiver)?;
        let method_arg_cts = extract_arg_concrete_types(self, args);
        let mut combined_arg_types: Vec<Option<shape_value::v2::ConcreteType>> =
            Vec::with_capacity(1 + method_arg_cts.len());
        combined_arg_types.push(Some(receiver_ct));
        combined_arg_types.extend(method_arg_cts);

        // 3. Resolve type parameter bindings from the call site.
        let resolution =
            resolve_call_site_type_args(self, func_name, &combined_arg_types, &type_params)?;

        // 4. All type args must be concrete (no unresolved variables).
        if resolution.type_args.is_empty() {
            return None;
        }

        // 5. Call ensure_monomorphic_function to get/create the specialization.
        //    On failure, return None to fall back to the generic version.
        match self.ensure_monomorphic_function(func_name, &resolution.type_args) {
            Ok(specialized_idx) => {
                let idx = specialized_idx as usize;
                // Self-call guard: if the monomorphized specialization is the
                // same function we are currently compiling (e.g. `Vec.len::i64`
                // calling `self.len()` which monomorphizes back to itself),
                // return None so the caller falls through to the built-in
                // method dispatch, preventing infinite recursion at runtime.
                if self.current_function == Some(idx) {
                    return None;
                }
                Some(idx)
            }
            Err(_) => None,
        }
    }
}
