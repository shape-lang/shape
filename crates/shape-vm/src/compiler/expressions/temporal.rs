//! Temporal expression compilation (time, datetime, duration, timeframe)

use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use shape_ast::ast::Expr;
use shape_ast::error::Result;

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a time reference expression
    pub(super) fn compile_expr_time_ref(
        &mut self,
        time_ref: &shape_ast::ast::TimeReference,
    ) -> Result<()> {
        let const_idx = self
            .program
            .add_constant(Constant::TimeReference(time_ref.clone()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(const_idx)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::EvalTimeRef)),
        ));
        Ok(())
    }

    /// Compile a datetime expression
    pub(super) fn compile_expr_datetime(
        &mut self,
        datetime_expr: &shape_ast::ast::DateTimeExpr,
    ) -> Result<()> {
        let const_idx = self
            .program
            .add_constant(Constant::DateTimeExpr(datetime_expr.clone()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(const_idx)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::EvalDateTimeExpr)),
        ));
        Ok(())
    }

    /// Compile a duration expression
    pub(super) fn compile_expr_duration(
        &mut self,
        duration: &shape_ast::ast::Duration,
    ) -> Result<()> {
        let const_idx = self
            .program
            .add_constant(Constant::Duration(duration.clone()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(const_idx)),
        ));
        Ok(())
    }

    /// Compile a timeframe context expression
    pub(super) fn compile_expr_timeframe_context(
        &mut self,
        timeframe: shape_ast::ast::Timeframe,
        expr: &Expr,
    ) -> Result<()> {
        let tf_const = self.program.add_constant(Constant::Timeframe(timeframe));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(tf_const)),
        ));
        self.emit(Instruction::simple(OpCode::PushTimeframe));
        self.compile_expr(expr)?;
        self.emit(Instruction::simple(OpCode::PopTimeframe));
        Ok(())
    }
}
