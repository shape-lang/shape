//! Stack operations for the VM executor
//!
//! Handles basic stack manipulation: push, pop, dup, swap

use std::sync::Arc;

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
};
use shape_value::{VMError, ValueWord, ValueWordExt};
impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_stack_ops(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            PushConst => self.op_push_const(instruction)?,
            PushNull => self.push_raw_u64(ValueWord::none())?,
            Pop => {
                self.pop_raw_u64()?;
            }
            Dup => {
                // Clone the ValueWord directly from the stack, avoiding ValueWord round-trip
                let index = self.sp.checked_sub(1).ok_or(VMError::StackUnderflow)?;
                if index >= self.stack.len() {
                    return Err(VMError::StackUnderflow);
                }
                let val = self.stack_read_raw(index);
                self.push_raw_u64(val)?;
            }
            Swap => {
                let b = self.pop_raw_u64()?;
                let a = self.pop_raw_u64()?;
                self.push_raw_u64(b)?;
                self.push_raw_u64(a)?;
            }
            PromoteToOwned => self.op_promote_to_owned()?,
            // Phase 5.C: `ReturnOwned` has identical runtime semantics to
            // `PromoteToOwned` — it converts a freshly-allocated Arc on the
            // return value to a Box. Emitted by callees whose
            // `ReturnOwnershipMode` is `NewlyOwned` so callers can skip their
            // own promotion after the call returns.
            ReturnOwned => self.op_promote_to_owned()?,
            // V1.2B: inverse of `PromoteToOwned` — converts a Box-owned
            // heap value on top-of-stack into an Arc-shared one. No-op
            // for inline scalars and already-shared heap values.
            PromoteToShared => self.op_promote_to_shared()?,
            _ => unreachable!(
                "exec_stack_ops called with non-stack opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    /// Promote the top-of-stack value from shared (Arc) to owned (Box) allocation
    /// if the heap refcount is exactly 1 (sole owner). This is a safe optimization:
    /// - Inline values (int, float, bool, null): no-op.
    /// - Already-owned heap values: no-op.
    /// - Shared heap values with refcount > 1: no-op (cannot convert safely).
    /// - Shared heap values with refcount == 1: Arc is unwrapped into Box.
    ///
    /// When the `gc` feature is enabled, ownership is managed by the GC, so
    /// this is a no-op.
    #[inline(always)]
    fn op_promote_to_owned(&mut self) -> Result<(), VMError> {
        #[cfg(feature = "gc")]
        {
            return Ok(());
        }

        #[cfg(not(feature = "gc"))]
        {
            use shape_value::tag_bits::{
                get_payload, get_tag, is_tagged, HEAP_OWNED_BIT, HEAP_PTR_MASK, TAG_HEAP,
            };
            use shape_value::heap_value::HeapValue;

            let index = self.sp.checked_sub(1).ok_or(VMError::StackUnderflow)?;
            let bits = self.stack[index];

            // Fast exit: not a heap-tagged value (inline scalar, function, etc.)
            if !is_tagged(bits) || get_tag(bits) != TAG_HEAP {
                return Ok(());
            }

            let payload = get_payload(bits);

            // Already owned — nothing to do
            if (payload & HEAP_OWNED_BIT) != 0 {
                return Ok(());
            }

            // Shared (Arc-backed). Check if we are the sole owner.
            let ptr = (payload & HEAP_PTR_MASK) as *const HeapValue;
            if ptr.is_null() {
                return Ok(());
            }

            // Reconstruct Arc without consuming it (ManuallyDrop prevents decrement).
            let arc = std::mem::ManuallyDrop::new(unsafe { Arc::from_raw(ptr) });
            if Arc::strong_count(&arc) == 1 {
                // Sole owner — safe to convert Arc -> Box.
                let arc = std::mem::ManuallyDrop::into_inner(arc);
                match Arc::try_unwrap(arc) {
                    Ok(hv) => {
                        let new_bits = shape_value::ValueBits::heap_box_owned(hv).raw();
                        // Replace top of stack in-place (no push/pop needed).
                        self.stack[index] = new_bits;
                    }
                    Err(_arc) => {
                        // Race: someone else got a ref between check and unwrap.
                        // Err(arc) returns the Arc; its Drop will decrement.
                        // The stack still holds the original bits expecting an
                        // Arc refcount, so bump to compensate for the drop.
                        let raw_ptr = (payload & HEAP_PTR_MASK) as *const HeapValue;
                        unsafe { Arc::increment_strong_count(raw_ptr); }
                    }
                }
            }
            // If refcount > 1, leave as shared. ManuallyDrop prevents decrement.
            Ok(())
        }
    }

    /// Promote the top-of-stack value from owned (Box) to shared (Arc)
    /// allocation — the inverse of `op_promote_to_owned`.
    ///
    /// - Inline values (int, float, bool, null): no-op.
    /// - Already-shared heap values (Arc, owned bit clear): no-op.
    /// - Owned heap values (Box, owned bit set): the Box is reclaimed, its
    ///   inner `HeapValue` is moved into a freshly-allocated `Arc`, and the
    ///   resulting ValueWord (owned bit clear) replaces the top of stack.
    ///
    /// Stack effect: 0 pops / 0 pushes (TOS mutated in place), identical
    /// to `PromoteToOwned`.
    ///
    /// When the `gc` feature is enabled, ownership is managed by the GC,
    /// so this is a no-op.
    #[inline(always)]
    fn op_promote_to_shared(&mut self) -> Result<(), VMError> {
        #[cfg(feature = "gc")]
        {
            return Ok(());
        }

        #[cfg(not(feature = "gc"))]
        {
            use shape_value::heap_value::HeapValue;
            use shape_value::tag_bits::{
                get_payload, get_tag, is_tagged, HEAP_OWNED_BIT, HEAP_PTR_MASK, TAG_HEAP,
            };

            let index = self.sp.checked_sub(1).ok_or(VMError::StackUnderflow)?;
            let bits = self.stack[index];

            // Fast exit: not a heap-tagged value (inline scalar, function, etc.)
            if !is_tagged(bits) || get_tag(bits) != TAG_HEAP {
                return Ok(());
            }

            let payload = get_payload(bits);

            // Already shared (Arc-backed) — nothing to do.
            if (payload & HEAP_OWNED_BIT) == 0 {
                return Ok(());
            }

            // Owned (Box-backed). Reclaim the Box, move the inner value
            // into a fresh Arc, and replace TOS with the new ValueWord.
            let ptr = (payload & HEAP_PTR_MASK) as *mut HeapValue;
            if ptr.is_null() {
                return Ok(());
            }

            // SAFETY: the top-of-stack bits encode a `HEAP_OWNED_BIT`-set
            // ValueWord produced by `vw_heap_box_owned`, which in turn
            // called `Box::into_raw`. The VM stack slot is the sole
            // owner of this allocation (by construction of the owned
            // bit — unique ownership) and we are consuming it here. The
            // subsequent overwrite of `self.stack[index]` transfers
            // ownership out of the Box reconstruction so no
            // double-free occurs. Mirror of the unsafe block in
            // `op_promote_to_owned` that performs the Arc→Box
            // conversion.
            let boxed: Box<HeapValue> = unsafe { Box::from_raw(ptr) };
            let inner: HeapValue = *boxed;
            // `ValueWord::heap_box` wraps the HeapValue in an Arc and
            // emits a TAG_HEAP ValueWord with the owned bit clear —
            // the canonical shared encoding.
            let new_bits = ValueWord::heap_box(inner);
            self.stack[index] = new_bits;
            Ok(())
        }
    }

    pub(in crate::executor) fn op_push_const(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Const(idx)) = instruction.operand {
            let constant = self
                .program
                .constants
                .get(idx as usize)
                .ok_or(VMError::InvalidOperand)?;

            // Stage 2.2: For typed scalar constants (Number/Int/Bool), push the
            // raw bits directly via push_raw_* — skips the ValueWord wrapper
            // construction so downstream typed handlers (e.g. exec_typed_arithmetic)
            // can pop_raw_* without unwrapping. Encoding is identical to what
            // ValueWord::from_*() would produce, so legacy pop_vw consumers
            // (which transmute the raw bits back into a ValueWord) keep working.
            match constant {
                crate::bytecode::Constant::Number(n) => {
                    return self.push_raw_f64(*n);
                }
                crate::bytecode::Constant::Int(i) => {
                    // In-range i48: push raw tagged bits. Out-of-range falls
                    // back to ValueWord::from_i64 which heap-boxes as BigInt.
                    if *i >= shape_value::tag_bits::I48_MIN && *i <= shape_value::tag_bits::I48_MAX {
                        return self.push_raw_i64(*i);
                    }
                    return self.push_raw_u64(ValueWord::from_i64(*i));
                }
                crate::bytecode::Constant::UInt(u) => {
                    // In-range i48 (u <= I48_MAX): push raw tagged bits.
                    // Otherwise fall back to ValueWord constructors.
                    if *u <= shape_value::tag_bits::I48_MAX as u64 {
                        return self.push_raw_i64(*u as i64);
                    }
                    return if *u <= i64::MAX as u64 {
                        self.push_raw_u64(ValueWord::from_i64(*u as i64))
                    } else {
                        self.push_raw_u64(ValueWord::from_native_u64(*u))
                    };
                }
                crate::bytecode::Constant::Bool(b) => {
                    return self.push_raw_bool(*b);
                }
                crate::bytecode::Constant::Null => return self.push_raw_u64(ValueWord::none()),
                crate::bytecode::Constant::Unit => return self.push_raw_u64(ValueWord::unit()),
                crate::bytecode::Constant::Function(id) => {
                    return self.push_raw_u64(ValueWord::from_function(*id));
                }
                _ => {}
            }

            // For types with direct ValueWord constructors, skip ValueWord
            match constant {
                crate::bytecode::Constant::String(s) => {
                    return self.push_raw_u64(ValueWord::from_string(Arc::new(s.clone())));
                }
                crate::bytecode::Constant::Char(c) => {
                    return self.push_raw_u64(ValueWord::from_char(*c));
                }
                crate::bytecode::Constant::Decimal(d) => {
                    return self.push_raw_u64(ValueWord::from_decimal(*d));
                }
                _ => {}
            }

            // For remaining complex types, construct HeapValue directly (no ValueWord)
            use shape_value::heap_value::HeapValue;
            let heap_val = match constant {
                crate::bytecode::Constant::Timeframe(tf) => HeapValue::Temporal(shape_value::TemporalData::Timeframe(*tf)),
                crate::bytecode::Constant::Duration(duration) => {
                    // Convert AST Duration to chrono::Duration (TimeSpan) so it
                    // participates in DateTime arithmetic (Time +/- TimeSpan).
                    let chrono_dur =
                        crate::executor::builtins::datetime_builtins::ast_duration_to_chrono(
                            duration,
                        );
                    HeapValue::Temporal(shape_value::TemporalData::TimeSpan(chrono_dur))
                }
                crate::bytecode::Constant::TimeReference(time_ref) => {
                    HeapValue::Temporal(shape_value::TemporalData::TimeReference(Box::new(time_ref.clone())))
                }
                crate::bytecode::Constant::DateTimeExpr(expr) => {
                    HeapValue::Temporal(shape_value::TemporalData::DateTimeExpr(Box::new(expr.clone())))
                }
                crate::bytecode::Constant::DataDateTimeRef(expr) => {
                    HeapValue::Temporal(shape_value::TemporalData::DataDateTimeRef(Box::new(expr.clone())))
                }
                crate::bytecode::Constant::TypeAnnotation(type_annotation) => {
                    HeapValue::Rare(shape_value::RareHeapData::TypeAnnotation(Box::new(type_annotation.clone())))
                }
                crate::bytecode::Constant::Value(val) => {
                    return self.push_raw_u64(val.clone());
                }
                // Simple types and String/Decimal already handled above
                _ => unreachable!(),
            };

            self.push_raw_u64(ValueWord::from_heap_value(heap_val))?;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }
}

// ===== V1.2B: PromoteToShared tests =====
//
// These hand-crafted programs exercise the new `PromoteToShared` handler
// (`op_promote_to_shared`) in isolation. V1.2C will wire compiler
// emission; until then no user program produces this opcode, so these
// tests are the only coverage path.
//
// The handler is the inverse of `PromoteToOwned`: Box-owned heap values
// on TOS are reclaimed and re-wrapped as Arc (HEAP_OWNED_BIT cleared);
// inline scalars and already-Arc heap values are no-ops.
#[cfg(test)]
mod promote_to_shared_tests {
    use crate::bytecode::{BytecodeProgram, Constant, Instruction, OpCode, Operand};
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::{ValueWord, ValueWordExt};

    /// Helper: build a program, load it, execute, return the top-of-stack
    /// ValueWord (raw bits, already owned by the caller via `stack_take_raw`).
    fn run_program(program: BytecodeProgram) -> ValueWord {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm.execute(None).unwrap().clone()
    }

    #[test]
    fn test_promote_to_shared_on_inline_is_noop() {
        // Inline int (i48) should pass through PromoteToShared unchanged —
        // no heap path, bits preserved exactly.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(42));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::PromoteToShared),
            Instruction::simple(OpCode::Halt),
        ];
        let result = run_program(program);
        assert_eq!(
            result.as_i64(),
            Some(42),
            "inline int should round-trip through PromoteToShared",
        );
    }

    #[test]
    fn test_promote_to_shared_on_float_is_noop() {
        // Inline f64 also pass-through.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Number(3.14));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::PromoteToShared),
            Instruction::simple(OpCode::Halt),
        ];
        let result = run_program(program);
        assert_eq!(result.as_f64(), Some(3.14));
    }

    #[test]
    fn test_promote_to_shared_on_null_is_noop() {
        // Null passes through unchanged.
        let mut program = BytecodeProgram::default();
        program.instructions = vec![
            Instruction::simple(OpCode::PushNull),
            Instruction::simple(OpCode::PromoteToShared),
            Instruction::simple(OpCode::Halt),
        ];
        let result = run_program(program);
        assert!(result.is_none(), "null should survive PromoteToShared");
    }

    #[test]
    #[cfg(not(feature = "gc"))]
    fn test_promote_to_shared_on_arc_is_noop() {
        // An Arc-backed string (default PushConst String path) already has
        // HEAP_OWNED_BIT clear. PromoteToShared must leave it untouched —
        // same pointer, same refcount (1).
        use shape_value::ValueBits;

        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::String("arc-noop".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::PromoteToShared),
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap();
        let bits = result.raw_bits();
        let vb = ValueBits::from_raw(bits);
        assert!(
            !vb.is_heap_owned(),
            "Arc-backed value must remain shared after PromoteToShared",
        );
        assert!(
            vb.is_heap_shared(),
            "Arc-backed value must remain a heap-shared tag",
        );
        // Refcount is still 1 — the stack-top holds the sole live reference.
        let ptr = vb.heap_ptr();
        assert!(!ptr.is_null());
        // SAFETY: ptr is a valid Arc-backed heap pointer; ManuallyDrop keeps
        // the refcount unchanged for inspection.
        let arc = std::mem::ManuallyDrop::new(unsafe {
            std::sync::Arc::from_raw(ptr)
        });
        assert_eq!(
            std::sync::Arc::strong_count(&arc),
            1,
            "PromoteToShared on already-Arc must not bump refcount",
        );
        assert_eq!(
            result.as_str().map(|s| s.to_string()),
            Some("arc-noop".to_string()),
        );
    }

    #[test]
    #[cfg(not(feature = "gc"))]
    fn test_promote_to_shared_on_box_converts_to_arc() {
        // Produce a Box-owned string via PromoteToOwned, then convert it
        // back to Arc via PromoteToShared. After the round-trip, the
        // HEAP_OWNED_BIT must be clear and the value must be heap-shared.
        use shape_value::ValueBits;

        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::String("box-to-arc".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            // rc=1, Arc-backed
            Instruction::simple(OpCode::PromoteToOwned),
            // Box-backed (HEAP_OWNED_BIT set), rc does not apply
            Instruction::simple(OpCode::PromoteToShared),
            // Arc-backed again (HEAP_OWNED_BIT clear), rc=1
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap();
        let bits = result.raw_bits();
        let vb = ValueBits::from_raw(bits);

        assert!(
            !vb.is_heap_owned(),
            "after PromoteToShared the owned bit must be clear",
        );
        assert!(
            vb.is_heap_shared(),
            "after PromoteToShared the value must be heap-shared (Arc-backed)",
        );
        assert_eq!(
            result.as_str().map(|s| s.to_string()),
            Some("box-to-arc".to_string()),
            "string content must survive the Box→Arc conversion",
        );
    }

    #[test]
    #[cfg(not(feature = "gc"))]
    fn test_promote_to_shared_refcount_transfer() {
        // Deep correctness: after Box→Arc promotion, the resulting Arc has
        // strong_count == 1. This confirms a fresh Arc allocation (not a
        // smuggled existing one) and that the Box ownership was fully
        // consumed.
        use shape_value::ValueBits;

        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::String("rc-transfer".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::PromoteToOwned),
            Instruction::simple(OpCode::PromoteToShared),
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap();
        let bits = result.raw_bits();
        let vb = ValueBits::from_raw(bits);

        assert!(vb.is_heap_shared());
        let ptr = vb.heap_ptr();
        assert!(!ptr.is_null());
        // SAFETY: `bits` is a freshly produced Arc-backed heap ValueWord
        // from `PromoteToShared`. ManuallyDrop avoids altering the count.
        let arc = std::mem::ManuallyDrop::new(unsafe {
            std::sync::Arc::from_raw(ptr)
        });
        assert_eq!(
            std::sync::Arc::strong_count(&arc),
            1,
            "fresh Arc from Box→Arc promotion must have rc=1",
        );
    }

    #[test]
    #[cfg(not(feature = "gc"))]
    fn test_promote_to_shared_then_clone_shares_refcount() {
        // After Box→Arc promotion, `Dup` copies the raw bits — the Arc
        // pointer aliases. Follow with an explicit CloneLocal-equivalent
        // via StoreLocal + CloneLocal to bump the refcount, confirming
        // Arc semantics. The final stack-top is the clone; the original
        // Arc lives in slot 0, so strong_count must be 2.
        use shape_value::ValueBits;

        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::String("clone-after".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::PromoteToOwned),
            Instruction::simple(OpCode::PromoteToShared),
            // slot0 receives the Arc (rc=1)
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            // CloneLocal bumps the refcount (rc=2) and pushes onto TOS
            Instruction::new(OpCode::CloneLocal, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap();
        let bits = result.raw_bits();
        let vb = ValueBits::from_raw(bits);

        assert!(
            vb.is_heap_shared(),
            "CloneLocal of an Arc slot must yield a shared heap ref",
        );
        let ptr = vb.heap_ptr();
        assert!(!ptr.is_null());
        // SAFETY: `bits` is Arc-backed; ManuallyDrop preserves rc.
        let arc = std::mem::ManuallyDrop::new(unsafe {
            std::sync::Arc::from_raw(ptr)
        });
        assert_eq!(
            std::sync::Arc::strong_count(&arc),
            2,
            "CloneLocal should have bumped rc: slot0 + stack top = 2",
        );
        assert_eq!(
            result.as_str().map(|s| s.to_string()),
            Some("clone-after".to_string()),
        );
    }
}

