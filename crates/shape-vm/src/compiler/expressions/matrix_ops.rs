//! Typed matrix operator lowering.
//!
//! This module detects typed matrix multiplication forms and lowers:
//! - `Mat<number> * Vec<number>` -> `IntrinsicMatMulVec`
//! - `Mat<number> * Mat<number>` -> `IntrinsicMatMulMat`

use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::type_tracking::VariableTypeInfo;
use shape_ast::ast::{Expr, TypeAnnotation};
use shape_ast::error::Result;

use super::super::BytecodeCompiler;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MatMulKernel {
    MatVec,
    MatMat,
}

fn is_number_name(name: &str) -> bool {
    matches!(name.trim(), "number" | "Number" | "f64" | "float" | "Float")
}

fn parse_single_arg_generic<'a>(name: &'a str, base: &str) -> Option<&'a str> {
    let name = name.trim();
    let rest = name.strip_prefix(base)?.strip_prefix('<')?;
    let inner = rest.strip_suffix('>')?;
    Some(inner.trim())
}

fn is_vec_number_type_name(type_name: &str) -> bool {
    parse_single_arg_generic(type_name, "Vec").is_some_and(is_number_name)
}

fn is_mat_number_type_name(type_name: &str) -> bool {
    parse_single_arg_generic(type_name, "Mat").is_some_and(is_number_name)
}

impl BytecodeCompiler {
    fn expr_type_name_hint(&mut self, expr: &Expr) -> Option<String> {
        if let Expr::Identifier(name, _) = expr {
            if let Some(local_idx) = self.resolve_local(name)
                && let Some(type_name) = self
                    .type_tracker
                    .get_local_type(local_idx)
                    .and_then(|t| t.type_name.clone())
            {
                return Some(type_name);
            }

            let scoped_name = self
                .resolve_scoped_module_binding_name(name)
                .unwrap_or_else(|| name.clone());
            if let Some(binding_idx) = self.module_bindings.get(&scoped_name)
                && let Some(type_name) = self
                    .type_tracker
                    .get_binding_type(*binding_idx)
                    .and_then(|t| t.type_name.clone())
            {
                return Some(type_name);
            }
        }

        self.infer_expr_type(expr)
            .ok()
            .and_then(|t| t.to_annotation())
            .map(|ann| match ann {
                TypeAnnotation::Array(inner) => format!("Vec<{}>", inner.to_type_string()),
                _ => ann.to_type_string(),
            })
    }

    fn classify_typed_matrix_mul(&mut self, left: &Expr, right: &Expr) -> Option<MatMulKernel> {
        let left_ty = self.expr_type_name_hint(left)?;
        let right_ty = self.expr_type_name_hint(right)?;

        if is_mat_number_type_name(&left_ty) && is_vec_number_type_name(&right_ty) {
            return Some(MatMulKernel::MatVec);
        }
        if is_mat_number_type_name(&left_ty) && is_mat_number_type_name(&right_ty) {
            return Some(MatMulKernel::MatMat);
        }
        None
    }

    pub(super) fn try_compile_typed_matrix_mul(
        &mut self,
        left: &Expr,
        right: &Expr,
    ) -> Result<bool> {
        let Some(kernel) = self.classify_typed_matrix_mul(left, right) else {
            return Ok(false);
        };

        self.compile_expr(left)?;
        self.compile_expr(right)?;

        let arg_count = self.program.add_constant(Constant::Number(2.0));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(arg_count)),
        ));

        let builtin = match kernel {
            MatMulKernel::MatVec => BuiltinFunction::IntrinsicMatMulVec,
            MatMulKernel::MatMat => BuiltinFunction::IntrinsicMatMulMat,
        };
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(builtin)),
        ));

        self.last_expr_schema = None;
        self.last_expr_numeric_type = None;
        self.last_expr_type_info = Some(VariableTypeInfo::named(
            match kernel {
                MatMulKernel::MatVec => "Vec<number>",
                MatMulKernel::MatMat => "Mat<number>",
            }
            .to_string(),
        ));
        Ok(true)
    }
}
