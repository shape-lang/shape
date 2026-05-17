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

// C1-temporal-lowering (Phase 2d Wave 2): test bodies rewritten against
// the post-`ValueWord` carrier shape.
//
// The pre-strict-typing bodies asserted via deleted `KindedSlot` heap
// accessors `as_datetime()` / `as_timespan()` (forbidden per ADR-006
// §2.7.6/Q8 carrier-API bound — `KindedSlot` only exposes scalar
// accessors; heap variants dispatch through HeapValue match).
//
// The post-W11 surface used `slot.as_heap_value()` for that match, but
// for Temporal slots the bits are `Arc::into_raw::<TemporalData>` —
// NOT a `Box<HeapValue>` allocation. `as_heap_value()` would be wrong-
// type recovery per the 5-arm receiver-recovery soundness rule
// (CLAUDE.md / handover §0). The sound pattern, mirroring
// `objects/datetime_methods.rs::recv_temporal`, is to dereference
// `bits as *const TemporalData` directly.
//
// These bodies use that pattern with a single shared helper to keep
// the read site canonical. The W11 `deep-tests` gate is removed —
// the tests now compile and pass under the standard `cargo test
// -p shape-vm --lib` invocation (no feature flag required).
#[cfg(test)]
mod tests {
    use crate::test_utils::eval;
    use shape_value::{KindedSlot, NativeKind};
    use shape_value::heap_value::{HeapKind, TemporalData};

    /// Borrow `&TemporalData` from a Temporal-kinded `KindedSlot`.
    ///
    /// Mirrors `executor/objects/datetime_methods.rs::recv_temporal` and
    /// `executor/objects/mod.rs::resolve_method_handler`'s Temporal arm.
    /// The `KindedSlot` owns one strong-count share for the duration of
    /// `&KindedSlot`; the returned `&TemporalData` is bounded by that
    /// borrow.
    fn as_temporal(slot: &KindedSlot) -> &TemporalData {
        assert_eq!(
            slot.kind,
            NativeKind::Ptr(HeapKind::Temporal),
            "expected Ptr(Temporal) kind, got {:?}",
            slot.kind,
        );
        let bits = slot.slot.raw();
        assert!(bits != 0, "Temporal slot bits are null");
        // SAFETY: per ADR-006 §2.4 / §2.7.4, `NativeKind::Ptr(HeapKind::Temporal)`
        // means `bits` is `Arc::into_raw::<TemporalData>` and `slot` owns one
        // strong-count share (so the inner `TemporalData` is alive).
        unsafe { &*(bits as *const TemporalData) }
    }

    fn expect_datetime(slot: &KindedSlot) -> chrono::DateTime<chrono::FixedOffset> {
        match as_temporal(slot) {
            TemporalData::DateTime(dt) => *dt,
            other => panic!("expected DateTime, got {}", other.type_name()),
        }
    }

    fn expect_timespan(slot: &KindedSlot) -> chrono::Duration {
        match as_temporal(slot) {
            TemporalData::TimeSpan(ts) => *ts,
            other => panic!("expected TimeSpan, got {}", other.type_name()),
        }
    }

    // === MED-11: @"..." DateTime literals ===

    #[test]
    fn test_datetime_literal_iso8601() {
        let result = eval(r#"@"2024-06-15T14:30:00+00:00""#);
        let dt = expect_datetime(&result);
        // 2024-06-15T14:30:00 UTC
        assert_eq!(dt.timestamp(), 1718461800);
    }

    #[test]
    fn test_datetime_literal_date_only() {
        let result = eval(r#"@"2024-01-15""#);
        let dt = expect_datetime(&result);
        // 2024-01-15 at midnight UTC
        assert_eq!(dt.timestamp(), 1705276800);
    }

    #[test]
    fn test_datetime_literal_datetime_no_tz() {
        let result = eval(r#"@"2024-06-15T14:30:00""#);
        let dt = expect_datetime(&result);
        // Assumed UTC: 2024-06-15T14:30:00 UTC
        assert_eq!(dt.timestamp(), 1718461800);
    }

    #[test]
    fn test_datetime_literal_in_fn() {
        let result = eval(
            r#"
            fn get_dt() {
                @"2024-01-15"
            }
            get_dt()
            "#,
        );
        let dt = expect_datetime(&result);
        assert_eq!(dt.timestamp(), 1705276800);
    }

    #[test]
    fn test_datetime_named_now() {
        let result = eval("@now");
        let dt = expect_datetime(&result);
        // Just check it's a reasonable timestamp (after 2024-01-01)
        assert!(dt.timestamp() > 1704067200);
    }

    #[test]
    fn test_datetime_named_today() {
        use chrono::Timelike;
        let result = eval("@today");
        let dt = expect_datetime(&result);
        // Should be midnight today, timestamp > 2024-01-01
        assert!(dt.timestamp() > 1704067200);
        // Verify it's at midnight (seconds within the day should be 0)
        assert_eq!(dt.hour(), 0);
        assert_eq!(dt.minute(), 0);
        assert_eq!(dt.second(), 0);
    }

    // === MED-12: Duration suffix arithmetic ===

    #[test]
    fn test_duration_value_exists() {
        // Duration should produce a TimeSpan value (not crash)
        let result = eval("3d");
        let ts = expect_timespan(&result);
        // 3 days = 259200 seconds
        assert_eq!(ts.num_seconds(), 259200);
    }

    // The pre-strict-typing forms of these three tests used `let dt =
    // @"…"; let dur = 3d; dt + dur` inside a fn body. Local `let` of a
    // Temporal-kinded slot in a fn scope routes through `DropCall` at
    // scope exit, which is a cross-cluster SURFACE owned by playbook §10
    // D-trait-obj (`executor/trait_object_ops.rs:123` —
    // `TypeName::drop` lookup depends on deleted `ValueWord::type_name`
    // / `ValueWord::from_function` / `call_value_immediate_nb` ABI plus
    // `raw_helpers::extract_io_handle`'s deleted tag_bits dispatch; the
    // receiver kind for the dispatched drop fn cannot be sourced from
    // the current opcode shape).
    //
    // C1-temporal-lowering's territory is the temporal-carrier lowering
    // (push/pop/dispatch/print), not the Drop opcode body. To exercise
    // the arithmetic without firing the unrelated DropCall SURFACE,
    // these tests use the literal-returning-fn pattern (same shape as
    // `test_datetime_subtraction_yields_timespan` which has worked since
    // the original wave-β migration). Once D-trait-obj closes the Drop
    // SURFACE, the let-in-fn bodies can be restored at zero arithmetic
    // cost — the carrier shape is identical.

    #[test]
    fn test_datetime_plus_duration_days() {
        let result = eval(
            r#"
            fn make_dt() { @"2024-01-15" }
            fn make_dur() { 3d }
            fn test() { make_dt() + make_dur() }
            test()
            "#,
        );
        let dt = expect_datetime(&result);
        // 2024-01-15 + 3 days = 2024-01-18 at midnight UTC
        // 1705276800 + 259200 = 1705536000
        assert_eq!(dt.timestamp(), 1705536000);
    }

    #[test]
    fn test_datetime_plus_duration_hours() {
        let result = eval(
            r#"
            fn make_dt() { @"2024-01-15" }
            fn make_dur() { 2h }
            fn test() { make_dt() + make_dur() }
            test()
            "#,
        );
        let dt = expect_datetime(&result);
        // 2024-01-15 midnight + 2 hours = 1705276800 + 7200
        assert_eq!(dt.timestamp(), 1705284000);
    }

    #[test]
    fn test_datetime_minus_duration() {
        let result = eval(
            r#"
            fn make_dt() { @"2024-01-15" }
            fn make_dur() { 1d }
            fn test() { make_dt() - make_dur() }
            test()
            "#,
        );
        let dt = expect_datetime(&result);
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
        let ts = expect_timespan(&result);
        // 5 days = 432000 seconds
        assert_eq!(ts.num_seconds(), 432000);
    }

    #[test]
    fn test_duration_seconds() {
        let result = eval("10s");
        let ts = expect_timespan(&result);
        assert_eq!(ts.num_seconds(), 10);
    }

    #[test]
    fn test_duration_minutes() {
        let result = eval("30m");
        let ts = expect_timespan(&result);
        assert_eq!(ts.num_seconds(), 1800);
    }

    #[test]
    fn test_duration_addition() {
        // Same DropCall-SURFACE consideration as the three
        // `test_datetime_plus_duration_*` / `_minus_duration` tests
        // above: switch from `let a = …; let b = …; a + b` (inside a
        // fn) to the return-from-fn pattern to keep arithmetic in
        // territory.
        let result = eval(
            r#"
            fn make_a() { 3d }
            fn make_b() { 2d }
            fn test() { make_a() + make_b() }
            test()
            "#,
        );
        let ts = expect_timespan(&result);
        // 5 days = 432000 seconds
        assert_eq!(ts.num_seconds(), 432000);
    }
}
