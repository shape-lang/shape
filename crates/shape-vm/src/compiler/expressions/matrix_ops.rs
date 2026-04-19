//! Typed matrix operator lowering.
//!
//! This module detects typed matrix multiplication forms and lowers:
//! - `Mat<number> * Vec<number>` -> `IntrinsicMatMulVec`
//! - `Mat<number> * Mat<number>` -> `IntrinsicMatMulMat`
//!
//! R5.4E extends this to element-wise matrix and vector arithmetic:
//! - `Mat<number> + Mat<number>`   -> `IntrinsicMatAdd`
//! - `Mat<number> - Mat<number>`   -> `IntrinsicMatSub`
//! - `Vec<number> + Vec<number>`   -> `IntrinsicVecAdd`
//! - `Vec<number> - Vec<number>`   -> `IntrinsicVecSub`
//! - `Vec<number> * Vec<number>`   -> `IntrinsicVecMul`
//! - `Vec<number> / Vec<number>`   -> `IntrinsicVecDiv`
//! - `Vec<int>    + Vec<int>`      -> `IntrinsicVecAddI64`
//!
//! These retargets bypass the dynamic arithmetic fallback for the seven
//! operand shapes pinned by the R5.4A baseline test.

use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::type_tracking::VariableTypeInfo;
use shape_ast::ast::{BinaryOp, Expr, TypeAnnotation};
use shape_ast::error::Result;

use super::super::BytecodeCompiler;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MatMulKernel {
    MatVec,
    MatMat,
}

/// Which element-wise matrix arithmetic kernel to dispatch to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MatArithKernel {
    Add,
    Sub,
}

/// Which element-wise vector arithmetic kernel to dispatch to. The I64Add
/// variant covers `Vec<int> + Vec<int>` and preserves overflow-error
/// semantics via `simd_vec_add_i64`; the remaining four cover the
/// `Vec<number>` cases for +/-/*//.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VecArithKernel {
    Add,
    Sub,
    Mul,
    Div,
    I64Add,
}

fn is_number_name(name: &str) -> bool {
    matches!(name.trim(), "number" | "Number" | "f64" | "float" | "Float")
}

fn is_int_name(name: &str) -> bool {
    matches!(
        name.trim(),
        "int" | "Int" | "Integer" | "i64" | "i32" | "i16" | "i8"
    )
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

fn is_vec_int_type_name(type_name: &str) -> bool {
    parse_single_arg_generic(type_name, "Vec").is_some_and(is_int_name)
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

    /// Classify an element-wise matrix arithmetic form. Returns the kernel
    /// when both operands resolve to `Mat<number>` and `op` is Add/Sub.
    fn classify_typed_matrix_arithmetic(
        &mut self,
        op: &BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> Option<MatArithKernel> {
        let kernel = match op {
            BinaryOp::Add => MatArithKernel::Add,
            BinaryOp::Sub => MatArithKernel::Sub,
            _ => return None,
        };
        let left_ty = self.expr_type_name_hint(left)?;
        let right_ty = self.expr_type_name_hint(right)?;
        if is_mat_number_type_name(&left_ty) && is_mat_number_type_name(&right_ty) {
            Some(kernel)
        } else {
            None
        }
    }

    /// R5.4E: retarget `Mat<number> + Mat<number>` / `Mat<number> - Mat<number>`
    /// at compile time to `IntrinsicMatAdd` / `IntrinsicMatSub`. Returns
    /// `Ok(true)` when emission happened, `Ok(false)` otherwise.
    ///
    /// Mirrors the shape of `try_compile_typed_matrix_mul`: both operands
    /// are compiled, an arg-count constant is pushed, then `BuiltinCall`
    /// dispatches to the intrinsic. The result type hint is preserved as
    /// `Mat<number>` so chained expressions keep their type shape.
    pub(super) fn try_compile_typed_matrix_arithmetic(
        &mut self,
        op: &BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> Result<bool> {
        let Some(kernel) = self.classify_typed_matrix_arithmetic(op, left, right) else {
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
            MatArithKernel::Add => BuiltinFunction::IntrinsicMatAdd,
            MatArithKernel::Sub => BuiltinFunction::IntrinsicMatSub,
        };
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(builtin)),
        ));

        self.last_expr_schema = None;
        self.last_expr_numeric_type = None;
        self.last_expr_type_info = Some(VariableTypeInfo::named("Mat<number>".to_string()));
        Ok(true)
    }

    /// Classify an element-wise vector arithmetic form. Returns the kernel
    /// for:
    /// - `Vec<number>` +/-/*// `Vec<number>` (four kernels)
    /// - `Vec<int> + Vec<int>` (one kernel, Add only — other ops on
    ///   `Vec<int>` are not retargeted here and fall through to the
    ///   dynamic arithmetic path unchanged).
    fn classify_typed_vec_arithmetic(
        &mut self,
        op: &BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> Option<VecArithKernel> {
        let left_ty = self.expr_type_name_hint(left)?;
        let right_ty = self.expr_type_name_hint(right)?;

        // Vec<int> + Vec<int> — Add only.
        if matches!(op, BinaryOp::Add)
            && is_vec_int_type_name(&left_ty)
            && is_vec_int_type_name(&right_ty)
        {
            return Some(VecArithKernel::I64Add);
        }

        if is_vec_number_type_name(&left_ty) && is_vec_number_type_name(&right_ty) {
            return match op {
                BinaryOp::Add => Some(VecArithKernel::Add),
                BinaryOp::Sub => Some(VecArithKernel::Sub),
                BinaryOp::Mul => Some(VecArithKernel::Mul),
                BinaryOp::Div => Some(VecArithKernel::Div),
                _ => None,
            };
        }
        None
    }

    /// R5.4E: retarget typed vector arithmetic at compile time to the
    /// matching `IntrinsicVec*` builtin. Returns `Ok(true)` when emission
    /// happened, `Ok(false)` otherwise.
    ///
    /// The result type hint is `Vec<number>` for the number kernels and
    /// `Vec<int>` for `IntrinsicVecAddI64` — both match the HeapKind of
    /// the value the runtime intrinsic returns.
    pub(super) fn try_compile_typed_vec_arithmetic(
        &mut self,
        op: &BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> Result<bool> {
        let Some(kernel) = self.classify_typed_vec_arithmetic(op, left, right) else {
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
            VecArithKernel::Add => BuiltinFunction::IntrinsicVecAdd,
            VecArithKernel::Sub => BuiltinFunction::IntrinsicVecSub,
            VecArithKernel::Mul => BuiltinFunction::IntrinsicVecMul,
            VecArithKernel::Div => BuiltinFunction::IntrinsicVecDiv,
            VecArithKernel::I64Add => BuiltinFunction::IntrinsicVecAddI64,
        };
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(builtin)),
        ));

        self.last_expr_schema = None;
        self.last_expr_numeric_type = None;
        self.last_expr_type_info = Some(VariableTypeInfo::named(
            match kernel {
                VecArithKernel::I64Add => "Vec<int>",
                _ => "Vec<number>",
            }
            .to_string(),
        ));
        Ok(true)
    }
}
