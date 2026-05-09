//! Stack operations for the VM executor (ADR-006 §2.7.7 / Q9 — kinded stack).
//!
//! Handles basic stack manipulation: PushConst, PushNull, Pop, Dup, Swap,
//! plus the legacy `PromoteToOwned` / `PromoteToShared` opcodes.
//!
//! Wave 6: every push/pop now threads through the kinded API
//! (`push_kinded(bits, kind)` / `pop_kinded()`). Kind is sourced from the
//! constant being pushed (compile-time-known per Constant variant). The
//! Box-vs-Arc heap promotion machinery (`PromoteToOwned`/`PromoteToShared`)
//! becomes a no-op: the kinded model carries `Arc<T>` directly per
//! `KindedSlot::from_*` constructors — there is no Box-owned encoding.

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::vm_impl::stack::{clone_with_kind, drop_with_kind},
    executor::VirtualMachine,
};
use shape_value::{NativeKind, VMError, heap_value::HeapKind};
use std::sync::Arc;

impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_stack_ops(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            PushConst => self.op_push_const(instruction)?,
            PushNull => self.push_kinded(0u64, NativeKind::Bool)?,
            Pop => {
                let (bits, kind) = self.pop_kinded()?;
                drop_with_kind(bits, kind);
            }
            Dup => {
                // WB2.4 retain-on-read: `Dup` produces an independent
                // owning share of the top-of-stack. Bump the heap refcount
                // via `clone_with_kind` so both stack slots own a share.
                let index = self.sp.checked_sub(1).ok_or(VMError::StackUnderflow)?;
                let (bits, kind) = self.stack_read_kinded_raw(index);
                clone_with_kind(bits, kind);
                self.push_kinded(bits, kind)?;
            }
            Swap => {
                let (b_bits, b_kind) = self.pop_kinded()?;
                let (a_bits, a_kind) = self.pop_kinded()?;
                self.push_kinded(b_bits, b_kind)?;
                self.push_kinded(a_bits, a_kind)?;
            }
            // ADR-006: heap values are always Arc-backed via `KindedSlot::from_*`
            // constructors. The pre-Wave-6 Box-owned encoding (HEAP_OWNED_BIT)
            // is gone; PromoteToOwned / PromoteToShared collapse to no-ops.
            // The opcodes are preserved for bytecode compatibility (existing
            // FunctionBlobs reference them); the runtime semantics are now
            // "ensure top-of-stack is Arc-backed" — already true by
            // construction.
            PromoteToOwned | ReturnOwned | PromoteToShared => {
                // No-op: the kinded model never produces Box-backed slots.
            }
            _ => unreachable!(
                "exec_stack_ops called with non-stack opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
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

            // Wave 6: the kind for each Constant variant is compile-time
            // known. Push raw bits + the corresponding NativeKind into the
            // parallel kinds track.
            match constant {
                crate::bytecode::Constant::Number(n) => {
                    let bits = if n.is_nan() {
                        f64::NAN.to_bits()
                    } else {
                        n.to_bits()
                    };
                    return self.push_kinded(bits, NativeKind::Float64);
                }
                crate::bytecode::Constant::Int(i) => {
                    return self.push_kinded(*i as u64, NativeKind::Int64);
                }
                crate::bytecode::Constant::UInt(u) => {
                    return self.push_kinded(*u, NativeKind::UInt64);
                }
                crate::bytecode::Constant::Bool(b) => {
                    return self.push_kinded(*b as u64, NativeKind::Bool);
                }
                // Null: zero bits, Bool kind (the §2.7 default sentinel —
                // Drop is a no-op).
                crate::bytecode::Constant::Null => {
                    return self.push_kinded(0u64, NativeKind::Bool);
                }
                // Unit: same shape as Null (no payload).
                crate::bytecode::Constant::Unit => {
                    return self.push_kinded(0u64, NativeKind::Bool);
                }
                crate::bytecode::Constant::Function(id) => {
                    // Function ID is an inline u16 stored in the lower bits.
                    return self.push_kinded(*id as u64, NativeKind::UInt64);
                }
                _ => {}
            }

            // Heap-bearing constants: construct the matching Arc<T> and
            // push raw pointer bits with the per-kind discriminator.
            match constant {
                crate::bytecode::Constant::String(s) => {
                    let arc: Arc<String> = Arc::new(s.clone());
                    let bits = Arc::into_raw(arc) as u64;
                    return self.push_kinded(bits, NativeKind::String);
                }
                crate::bytecode::Constant::Char(c) => {
                    // Char: inline-scalar payload tagged through HeapKind
                    // for dispatch uniformity (no Arc<T>).
                    return self
                        .push_kinded(*c as u64, NativeKind::Ptr(HeapKind::Char));
                }
                crate::bytecode::Constant::Decimal(d) => {
                    let arc: Arc<rust_decimal::Decimal> = Arc::new(*d);
                    let bits = Arc::into_raw(arc) as u64;
                    return self
                        .push_kinded(bits, NativeKind::Ptr(HeapKind::Decimal));
                }
                _ => {}
            }

            // Remaining complex constants (Timeframe, Duration, TimeReference,
            // DateTimeExpr, DataDateTimeRef, TypeAnnotation, Value): these
            // are deferred to a follow-up wave that aligns the constant
            // table with the kinded heap encoding. For now they are
            // unreachable in normal compilation paths — the constant
            // emitter doesn't produce them outside of legacy code paths
            // already broken by the ValueWord deletion.
            return Err(VMError::RuntimeError(format!(
                "unsupported constant variant in PushConst (Wave 6 follow-up): {:?}",
                std::mem::discriminant(constant)
            )));
        }
        Err(VMError::InvalidOperand)
    }
}
