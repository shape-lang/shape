//! Vector SIMD intrinsic builtin implementations
//!
//! Handles all IntrinsicVec* functions with SIMD acceleration

use crate::{bytecode::BuiltinFunction, executor::VirtualMachine};
use shape_runtime::context::ExecutionContext;
use shape_value::VMError;

impl VirtualMachine {
    /// Handle vector intrinsic builtins (11 variants)
    /// All delegate to shape_runtime::intrinsics::vector::* functions
    pub(in crate::executor) fn handle_vector_intrinsic(
        &mut self,
        builtin: BuiltinFunction,
        ctx: Option<&mut ExecutionContext>,
    ) -> Result<(), VMError> {
        let nb_args = self.pop_builtin_args()?;

        let mut dummy_ctx = ExecutionContext::new_empty();
        let exec_ctx = ctx.unwrap_or(&mut dummy_ctx);

        use shape_runtime::intrinsics::vector;

        let result = match builtin {
            BuiltinFunction::IntrinsicVecAbs => vector::intrinsic_vec_abs(&nb_args, exec_ctx),
            BuiltinFunction::IntrinsicVecSqrt => vector::intrinsic_vec_sqrt(&nb_args, exec_ctx),
            BuiltinFunction::IntrinsicVecLn => vector::intrinsic_vec_ln(&nb_args, exec_ctx),
            BuiltinFunction::IntrinsicVecExp => vector::intrinsic_vec_exp(&nb_args, exec_ctx),
            BuiltinFunction::IntrinsicVecAdd => vector::intrinsic_vec_add(&nb_args, exec_ctx),
            BuiltinFunction::IntrinsicVecSub => vector::intrinsic_vec_sub(&nb_args, exec_ctx),
            BuiltinFunction::IntrinsicVecMul => vector::intrinsic_vec_mul(&nb_args, exec_ctx),
            BuiltinFunction::IntrinsicVecDiv => vector::intrinsic_vec_div(&nb_args, exec_ctx),
            BuiltinFunction::IntrinsicVecMax => vector::intrinsic_vec_max(&nb_args, exec_ctx),
            BuiltinFunction::IntrinsicVecMin => vector::intrinsic_vec_min(&nb_args, exec_ctx),
            BuiltinFunction::IntrinsicVecSelect => vector::intrinsic_vec_select(&nb_args, exec_ctx),
            _ => unreachable!("Not a vector intrinsic: {:?}", builtin),
        }
        .map_err(|e| VMError::RuntimeError(e.to_string()))?;

        self.push_raw_u64(result)?;
        Ok(())
    }
}
