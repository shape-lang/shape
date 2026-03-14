//! Type operation expression compilation

use crate::bytecode::{Constant, Instruction, NumericWidth, OpCode, Operand};
use shape_ast::ast::{Expr, Spanned, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_system::{Type, annotation_to_string};
use std::collections::HashSet;

use super::super::BytecodeCompiler;

const INTO_DISPATCH_TAG: &str = "__IntoDispatch";
const TRY_INTO_DISPATCH_TAG: &str = "__TryIntoDispatch";

fn type_name_to_annotation(name: &str) -> TypeAnnotation {
    match name {
        "number" | "int" | "decimal" | "string" | "bool" | "row" | "pattern" | "function"
        | "module_function" | "duration" | "datetime" | "time" | "timeframe" | "table"
        | "array" | "object" | "option" | "result" | "Type" | "type" | "i8" | "u8" | "i16"
        | "u16" | "i32" | "u32" | "i64" | "u64" | "isize" | "usize" | "byte" | "char" => {
            TypeAnnotation::Basic(name.to_string())
        }
        "()" | "unit" => TypeAnnotation::Void,
        "None" => TypeAnnotation::Null,
        _ => TypeAnnotation::Reference(name.into()),
    }
}

impl BytecodeCompiler {
    fn current_function_type_params(&self) -> HashSet<String> {
        let Some(func_idx) = self.current_function else {
            return HashSet::new();
        };
        let Some(func) = self.program.functions.get(func_idx) else {
            return HashSet::new();
        };
        let Some(func_def) = self.function_defs.get(&func.name) else {
            return HashSet::new();
        };

        func_def
            .type_params
            .as_ref()
            .map(|params| params.iter().map(|p| p.name.clone()).collect())
            .unwrap_or_default()
    }

    fn annotation_contains_type_param(ann: &TypeAnnotation, type_params: &HashSet<String>) -> bool {
        match ann {
            TypeAnnotation::Basic(name) => type_params.contains(name),
            TypeAnnotation::Reference(name) => type_params.contains(name.as_str()),
            TypeAnnotation::Array(inner) => {
                Self::annotation_contains_type_param(inner, type_params)
            }
            TypeAnnotation::Tuple(items)
            | TypeAnnotation::Union(items)
            | TypeAnnotation::Intersection(items) => items
                .iter()
                .any(|item| Self::annotation_contains_type_param(item, type_params)),
            TypeAnnotation::Object(fields) => fields.iter().any(|field| {
                Self::annotation_contains_type_param(&field.type_annotation, type_params)
            }),
            TypeAnnotation::Function { params, returns } => {
                params
                    .iter()
                    .any(|p| Self::annotation_contains_type_param(&p.type_annotation, type_params))
                    || Self::annotation_contains_type_param(returns, type_params)
            }
            TypeAnnotation::Generic { name, args } => {
                type_params.contains(name.as_str())
                    || args
                        .iter()
                        .any(|arg| Self::annotation_contains_type_param(arg, type_params))
            }
            TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined
            | TypeAnnotation::Dyn(_) => false,
        }
    }

    pub(super) fn should_runtime_type_query(&self, ann: &TypeAnnotation) -> bool {
        let type_params = self.current_function_type_params();
        if type_params.is_empty() {
            return false;
        }
        Self::annotation_contains_type_param(ann, &type_params)
    }

    pub(super) fn is_type_symbol_name(&self, name: &str) -> bool {
        self.struct_types.contains_key(name)
            || self.type_aliases.contains_key(name)
            || self.type_inference.env.lookup_type_alias(name).is_some()
            || self.type_inference.env.get_enum(name).is_some()
            || self.type_inference.env.lookup_interface(name).is_some()
            || self.type_inference.env.lookup_trait(name).is_some()
    }

    fn canonical_try_into_name(name: &str) -> String {
        match name {
            "boolean" | "Boolean" | "Bool" => "bool".to_string(),
            "String" => "string".to_string(),
            "Number" => "number".to_string(),
            "Int" => "int".to_string(),
            "Decimal" => "decimal".to_string(),
            _ => name.to_string(),
        }
    }

    fn try_into_name_from_annotation(annotation: &TypeAnnotation) -> Option<String> {
        match annotation {
            TypeAnnotation::Basic(name) => Some(Self::canonical_try_into_name(name)),
            TypeAnnotation::Reference(name) => Some(Self::canonical_try_into_name(name)),
            TypeAnnotation::Generic { name, .. } => Some(Self::canonical_try_into_name(name)),
            _ => None,
        }
    }

    fn try_into_name_from_type(ty: &Type) -> Option<String> {
        match ty {
            Type::Concrete(TypeAnnotation::Basic(name)) => {
                Some(Self::canonical_try_into_name(name))
            }
            Type::Concrete(TypeAnnotation::Reference(name)) => {
                Some(Self::canonical_try_into_name(name))
            }
            Type::Concrete(TypeAnnotation::Generic { name, .. }) => {
                Some(Self::canonical_try_into_name(name))
            }
            Type::Generic { base, .. } => match base.as_ref() {
                Type::Concrete(TypeAnnotation::Basic(name)) => {
                    Some(Self::canonical_try_into_name(name))
                }
                Type::Concrete(TypeAnnotation::Reference(name)) => {
                    Some(Self::canonical_try_into_name(name))
                }
                Type::Concrete(TypeAnnotation::Generic { name, .. }) => {
                    Some(Self::canonical_try_into_name(name))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn conversion_dispatch_annotation(
        tag: &str,
        source_name: &str,
        target_selector: &str,
    ) -> TypeAnnotation {
        TypeAnnotation::Generic {
            name: tag.into(),
            args: vec![
                TypeAnnotation::Reference(source_name.into()),
                TypeAnnotation::Reference(target_selector.into()),
            ],
        }
    }

    pub(super) fn expr_is_type_symbol(&self, expr: &Expr) -> bool {
        matches!(expr, Expr::Identifier(name, _) if self.is_type_symbol_name(name))
    }

    /// Resolve a static type annotation for an expression.
    ///
    /// This powers static `.type()` queries when the receiver type is fully
    /// known at compile time.
    pub(super) fn static_type_annotation_for_expr(
        &mut self,
        expr: &Expr,
    ) -> Result<TypeAnnotation> {
        // Support type symbols directly (e.g. Point.type()).
        if let Expr::Identifier(name, _) = expr {
            if self.is_type_symbol_name(name) {
                return Ok(TypeAnnotation::Reference(name.as_str().into()));
            }

            // Prefer compiler-tracked local/module_binding types for identifiers.
            if let Some(local_idx) = self.resolve_local(name) {
                if let Some(info) = self.type_tracker.get_local_type(local_idx) {
                    if let Some(type_name) = &info.type_name {
                        return Ok(type_name_to_annotation(type_name));
                    }
                }
            }
            if let Some(binding_idx) = self.module_bindings.get(name).copied() {
                if let Some(info) = self.type_tracker.get_binding_type(binding_idx) {
                    if let Some(type_name) = &info.type_name {
                        return Ok(type_name_to_annotation(type_name));
                    }
                }
            }
        }

        let inferred = self.infer_expr_type(expr)?;
        if let Some(annotation) = inferred.to_annotation() {
            return Ok(annotation);
        }

        Err(ShapeError::SemanticError {
            message: format!(
                "Could not resolve a concrete static type for expression in type query: {:?}",
                inferred
            ),
            location: Some(self.span_to_source_location(expr.span())),
        })
    }

    /// Compile a type assertion expression (expr as Type)
    ///
    /// This wraps the value with a TypeAnnotatedValue so that meta formatting
    /// can be applied when the value is printed.
    pub(super) fn compile_expr_type_assertion(
        &mut self,
        expr: &Expr,
        type_annotation: &shape_ast::ast::TypeAnnotation,
    ) -> Result<()> {
        if let TypeAnnotation::Generic { name, args } = type_annotation
            && name == "Option"
            && args.len() == 1
        {
            let inner_type = &args[0];
            let source_name = self
                .static_type_annotation_for_expr(expr)
                .ok()
                .and_then(|ann| Self::try_into_name_from_annotation(&ann))
                .or_else(|| {
                    self.infer_expr_type(expr)
                        .ok()
                        .and_then(|ty| Self::try_into_name_from_type(&ty))
                })
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "`as Type?` requires a concrete source type for TryInto dispatch, found expression '{}'",
                        annotation_to_string(type_annotation)
                    ),
                    location: Some(self.span_to_source_location(expr.span())),
                })?;
            let target_selector =
                Self::try_into_name_from_annotation(inner_type).ok_or_else(|| {
                    ShapeError::SemanticError {
                        message: format!(
                            "`as Type?` target must be a named type selector, found '{}'",
                            annotation_to_string(inner_type)
                        ),
                        location: Some(self.span_to_source_location(expr.span())),
                    }
                })?;

            // `as Type?` compiles to trait-dispatch metadata consumed by Convert.
            self.compile_expr(expr)?;
            let dispatch = Self::conversion_dispatch_annotation(
                TRY_INTO_DISPATCH_TAG,
                &source_name,
                &target_selector,
            );
            let target = self
                .program
                .add_constant(Constant::TypeAnnotation(dispatch));
            self.emit(Instruction::new(
                OpCode::Convert,
                Some(Operand::Const(target)),
            ));
            return Ok(());
        }

        // Width integer cast: `expr as i8`, `expr as u16`, etc.
        // Emits CastWidth which does bit-truncation (Rust-style).
        if let TypeAnnotation::Basic(name) = type_annotation {
            if let Some(w) = shape_ast::IntWidth::from_name(name) {
                self.compile_expr(expr)?;
                self.emit(Instruction::new(
                    OpCode::CastWidth,
                    Some(Operand::Width(NumericWidth::from_int_width(w))),
                ));
                self.last_expr_numeric_type = Some(crate::type_tracking::NumericType::IntWidth(w));
                return Ok(());
            }
        }

        if let Some(target_selector) = Self::try_into_name_from_annotation(type_annotation) {
            // `as Type` compiles to Into<Target> dispatch through Convert.
            self.compile_expr(expr)?;
            let dispatch = self
                .static_type_annotation_for_expr(expr)
                .ok()
                .and_then(|ann| Self::try_into_name_from_annotation(&ann))
                .or_else(|| {
                    self.infer_expr_type(expr)
                        .ok()
                        .and_then(|ty| Self::try_into_name_from_type(&ty))
                })
                .map(|source_name| {
                    Self::conversion_dispatch_annotation(
                        INTO_DISPATCH_TAG,
                        &source_name,
                        &target_selector,
                    )
                })
                .unwrap_or_else(|| type_annotation.clone());
            let target = self
                .program
                .add_constant(Constant::TypeAnnotation(dispatch));
            self.emit(Instruction::new(
                OpCode::Convert,
                Some(Operand::Const(target)),
            ));
            return Ok(());
        }

        // Compile the expression
        self.compile_expr(expr)?;

        // Get the type name for wrapping
        let type_name = annotation_to_string(type_annotation);

        // Add type name to string pool
        let type_name_idx = self.program.add_string(type_name);

        // Emit WrapTypeAnnotation to wrap the value
        self.emit(Instruction::new(
            OpCode::WrapTypeAnnotation,
            Some(Operand::Property(type_name_idx as u16)),
        ));

        Ok(())
    }

    /// Compile an explicit impl selector expression: `expr using ImplName`.
    ///
    /// We encode the selector as a TypeAnnotatedValue marker so formatting
    /// builtins can resolve named Display implementations at runtime.
    pub(super) fn compile_expr_using_impl(&mut self, expr: &Expr, impl_name: &str) -> Result<()> {
        self.compile_expr(expr)?;

        let marker = format!("__impl__:{}", impl_name);
        let marker_idx = self.program.add_string(marker);
        self.emit(Instruction::new(
            OpCode::WrapTypeAnnotation,
            Some(Operand::Property(marker_idx as u16)),
        ));
        Ok(())
    }

    /// Compile an instanceof expression
    pub(super) fn compile_expr_instanceof(
        &mut self,
        expr: &Expr,
        type_annotation: &shape_ast::ast::TypeAnnotation,
    ) -> Result<()> {
        self.compile_expr(expr)?;
        let type_const = self
            .program
            .add_constant(Constant::TypeAnnotation(type_annotation.clone()));
        self.emit(Instruction::new(
            OpCode::TypeCheck,
            Some(Operand::Const(type_const)),
        ));
        Ok(())
    }
}
