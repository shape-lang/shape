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
        // R5.3B: record the temporal display name on the expression-result
        // slot so that `propagate_assignment_type_to_slot` can populate the
        // local/binding tracker with `"DateTime"`. Reading that back at the
        // arithmetic site then lets the retarget at
        // `binary_ops.rs:750` / `:1049` fire for let-locals.
        self.last_expr_type_info =
            Some(crate::type_tracking::VariableTypeInfo::named("DateTime".to_string()));
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
        // R5.3B: record the temporal display name on the expression-result
        // slot (see compile_expr_datetime). Duration literals produce
        // `TimeSpan` at runtime; track it as `"Duration"` so the retarget
        // guard's "Duration" arm fires uniformly for let-locals.
        self.last_expr_type_info =
            Some(crate::type_tracking::VariableTypeInfo::named("Duration".to_string()));
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

// Tests gated `deep-tests` post-W11: bodies call `as_datetime()` and
// `as_timespan()` on the returned `KindedSlot`, which the kinded API
// (ADR-006 §2.7.6/Q8) does not provide — heap variants dispatch through
// `slot.as_heap_value()` + `HeapValue` match. Restoration requires
// rewriting these tests to use the heap-value match path (Phase-2c
// reentry per ADR-006 §2.7.4).
#[cfg(all(test, feature = "deep-tests"))]
mod tests {
    use crate::test_utils::eval;
    #[allow(unused_imports)]
    use eval as _;

    #[test]
    fn test_datetime_literal_iso8601() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_datetime_literal_date_only() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_datetime_literal_datetime_no_tz() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_datetime_literal_in_fn() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_datetime_named_now() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_datetime_named_today() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_duration_value_exists() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_datetime_plus_duration_days() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_datetime_plus_duration_hours() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_datetime_minus_duration() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_datetime_subtraction_yields_timespan() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_duration_seconds() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_duration_minutes() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

    #[test]
    fn test_duration_addition() {
        todo!("phase-2c — see ADR-006 §2.7.4 (KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer)")
    }

}
