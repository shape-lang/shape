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

#[cfg(test)]
mod tests {
    use crate::test_utils::eval;
    use shape_value::{ValueWord, ValueWordExt};

    // === MED-11: @"..." DateTime literals ===

    #[test]
    fn test_datetime_literal_iso8601() {
        let result = eval(r#"@"2024-06-15T14:30:00+00:00""#);
        let dt = result.as_datetime().expect("expected DateTime value");
        // 2024-06-15T14:30:00 UTC
        assert_eq!(dt.timestamp(), 1718461800);
    }

    #[test]
    fn test_datetime_literal_date_only() {
        let result = eval(r#"@"2024-01-15""#);
        let dt = result.as_datetime().expect("expected DateTime value");
        // 2024-01-15 at midnight UTC
        assert_eq!(dt.timestamp(), 1705276800);
    }

    #[test]
    fn test_datetime_literal_datetime_no_tz() {
        let result = eval(r#"@"2024-06-15T14:30:00""#);
        let dt = result.as_datetime().expect("expected DateTime value");
        // Assumed UTC: 2024-06-15T14:30:00 UTC
        assert_eq!(dt.timestamp(), 1718461800);
    }

    #[test]
    fn test_datetime_literal_in_fn() {
        // Use a function to test variable binding
        let result = eval(
            r#"
            fn get_dt() {
                @"2024-01-15"
            }
            get_dt()
            "#,
        );
        let dt = result.as_datetime().expect("expected DateTime value");
        assert_eq!(dt.timestamp(), 1705276800);
    }

    #[test]
    fn test_datetime_named_now() {
        let result = eval("@now");
        let dt = result.as_datetime().expect("expected DateTime value");
        // Just check it's a reasonable timestamp (after 2024-01-01)
        assert!(dt.timestamp() > 1704067200);
    }

    #[test]
    fn test_datetime_named_today() {
        let result = eval("@today");
        let dt = result.as_datetime().expect("expected DateTime value");
        // Should be midnight today, timestamp > 2024-01-01
        assert!(dt.timestamp() > 1704067200);
        // Verify it's at midnight (seconds within the day should be 0)
        use chrono::Timelike;
        assert_eq!(dt.hour(), 0);
        assert_eq!(dt.minute(), 0);
        assert_eq!(dt.second(), 0);
    }

    // === MED-12: Duration suffix arithmetic ===

    #[test]
    fn test_duration_value_exists() {
        // Duration should produce a TimeSpan value (not crash)
        let result = eval("3d");
        // Should be a TimeSpan (chrono::Duration)
        let ts = result.as_timespan().expect("expected TimeSpan value");
        // 3 days = 259200 seconds
        assert_eq!(ts.num_seconds(), 259200);
    }

    #[test]
    fn test_datetime_plus_duration_days() {
        let result = eval(
            r#"
            fn test() {
                let dt = @"2024-01-15"
                let dur = 3d
                dt + dur
            }
            test()
            "#,
        );
        let dt = result.as_datetime().expect("expected DateTime value");
        // 2024-01-15 + 3 days = 2024-01-18 at midnight UTC
        // 1705276800 + 259200 = 1705536000
        assert_eq!(dt.timestamp(), 1705536000);
    }

    #[test]
    fn test_datetime_plus_duration_hours() {
        let result = eval(
            r#"
            fn test() {
                let dt = @"2024-01-15"
                let dur = 2h
                dt + dur
            }
            test()
            "#,
        );
        let dt = result.as_datetime().expect("expected DateTime value");
        // 2024-01-15 midnight + 2 hours = 1705276800 + 7200
        assert_eq!(dt.timestamp(), 1705284000);
    }

    #[test]
    fn test_datetime_minus_duration() {
        let result = eval(
            r#"
            fn test() {
                let dt = @"2024-01-15"
                let dur = 1d
                dt - dur
            }
            test()
            "#,
        );
        let dt = result.as_datetime().expect("expected DateTime value");
        // 2024-01-15 - 1 day = 2024-01-14
        assert_eq!(dt.timestamp(), 1705190400);
    }

    #[test]
    fn test_datetime_subtraction_yields_timespan() {
        // Two datetime values subtracted should yield a TimeSpan
        let result = eval(
            r#"
            fn make_dt1() { @"2024-01-15" }
            fn make_dt2() { @"2024-01-10" }
            fn test() {
                make_dt1() - make_dt2()
            }
            test()
            "#,
        );
        let ts = result.as_timespan().expect("expected TimeSpan value");
        // 5 days = 432000 seconds
        assert_eq!(ts.num_seconds(), 432000);
    }

    #[test]
    fn test_duration_seconds() {
        let result = eval("10s");
        let ts = result.as_timespan().expect("expected TimeSpan value");
        assert_eq!(ts.num_seconds(), 10);
    }

    #[test]
    fn test_duration_minutes() {
        let result = eval("30m");
        let ts = result.as_timespan().expect("expected TimeSpan value");
        assert_eq!(ts.num_seconds(), 1800);
    }

    #[test]
    fn test_duration_addition() {
        let result = eval(
            r#"
            fn test() {
                let a = 3d
                let b = 2d
                a + b
            }
            test()
            "#,
        );
        let ts = result.as_timespan().expect("expected TimeSpan value");
        // 5 days = 432000 seconds
        assert_eq!(ts.num_seconds(), 432000);
    }
}
