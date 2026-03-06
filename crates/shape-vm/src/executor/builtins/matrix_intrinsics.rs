//! Matrix intrinsic builtin implementations.
//!
//! Handles matrix multiplication intrinsics and delegates to
//! `shape_runtime::intrinsics::matrix`.

use crate::{bytecode::BuiltinFunction, executor::VirtualMachine};
use shape_runtime::context::ExecutionContext;
use shape_value::VMError;

impl VirtualMachine {
    /// Handle matrix intrinsic builtins.
    pub(in crate::executor) fn handle_matrix_intrinsic(
        &mut self,
        builtin: BuiltinFunction,
        ctx: Option<&mut ExecutionContext>,
    ) -> Result<(), VMError> {
        let nb_args = self.pop_builtin_args()?;

        let mut dummy_ctx = ExecutionContext::new_empty();
        let exec_ctx = ctx.unwrap_or(&mut dummy_ctx);

        use shape_runtime::intrinsics::matrix;

        let result = match builtin {
            BuiltinFunction::IntrinsicMatMulVec => matrix::intrinsic_matmul_vec(&nb_args, exec_ctx),
            BuiltinFunction::IntrinsicMatMulMat => matrix::intrinsic_matmul_mat(&nb_args, exec_ctx),
            _ => unreachable!("Not a matrix intrinsic: {:?}", builtin),
        }
        .map_err(|e| VMError::RuntimeError(e.to_string()))?;

        self.push_vw(result)?;
        Ok(())
    }
}
