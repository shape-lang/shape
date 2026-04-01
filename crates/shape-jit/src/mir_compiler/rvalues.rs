//! Rvalue compilation: MIR Rvalue → Cranelift IR.
//!
//! Maps each Rvalue variant to Cranelift instructions:
//! - Use(operand): ownership-aware value load
//! - BinaryOp: arithmetic, comparison, logical operators
//! - UnaryOp: negation, logical not
//! - Clone: explicit clone (arc_retain)
//! - Borrow: reference creation (deferred)
//! - Aggregate: array/object construction

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::mir::types::*;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Compile an Rvalue to a Cranelift value.
    pub(crate) fn compile_rvalue(&mut self, rvalue: &Rvalue) -> Result<Value, String> {
        match rvalue {
            Rvalue::Use(operand) => self.compile_operand(operand),

            Rvalue::BinaryOp(op, lhs, rhs) => {
                // Check if both operands are i32-typed for v2 native i32 codegen.
                let lhs_i32 = self.operand_is_i32(lhs);
                let rhs_i32 = self.operand_is_i32(rhs);

                let l = self.compile_operand(lhs)?;
                let r = self.compile_operand(rhs)?;

                if lhs_i32 && rhs_i32 {
                    // Both operands are i32 — use native i32 instructions.
                    match op {
                        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                            self.compile_binop_i32(op, l, r)
                        }
                        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le
                        | BinOp::Gt | BinOp::Ge => {
                            self.compile_cmp_i32(op, l, r)
                        }
                        _ => self.compile_binop(op, l, r),
                    }
                } else {
                    self.compile_binop(op, l, r)
                }
            }

            Rvalue::UnaryOp(op, operand) => {
                let val = self.compile_operand(operand)?;
                self.compile_unop(op, val)
            }

            Rvalue::Clone(operand) => {
                // Explicit clone: get the value and retain.
                let val = self.compile_operand_raw(operand)?;
                self.builder.ins().call(self.ffi.arc_retain, &[val]);
                Ok(val)
            }

            Rvalue::Borrow(_kind, place) => {
                // StackSlot-based references (ported from BytecodeToIR):
                // 1. Read the current value from the place
                let val = self.read_place(place)?;
                // 2. Allocate an 8-byte stack slot to hold the referenced value
                let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    8,
                    3, // align = 8 (2^3)
                ));
                // 3. Store the value into the stack slot
                self.builder.ins().stack_store(val, slot, 0);
                // 4. Track the root local for reload-after-call
                let root = place.root_local();
                self.ref_stack_slots.insert(root, slot);
                // 5. Return the stack slot address as the reference value
                Ok(self.builder.ins().stack_addr(types::I64, slot, 0))
            }

            Rvalue::Aggregate(operands) => {
                // Create an empty array via jit_new_array(ctx, 0), then push elements.
                // Using count=0 avoids popping from ctx.stack — MirToIR compiles
                // operands directly instead of staging through the stack.
                let zero = self.builder.ins().iconst(types::I64, 0i64);
                let inst = self.builder.ins().call(
                    self.ffi.new_array,
                    &[self.ctx_ptr, zero],
                );
                let arr = self.builder.inst_results(inst)[0];

                for operand in operands {
                    let elem = self.compile_operand(operand)?;
                    self.builder
                        .ins()
                        .call(self.ffi.array_push_elem, &[arr, elem]);
                }

                Ok(arr)
            }
        }
    }

    /// Check if an operand references an i32-typed local slot.
    fn operand_is_i32(&self, operand: &Operand) -> bool {
        let place = match operand {
            Operand::Move(p) | Operand::MoveExplicit(p) | Operand::Copy(p) => p,
            Operand::Constant(_) => return false,
        };
        let slot = place.root_local();
        let kind = super::types::slot_kind_for_local(&self.slot_kinds, slot.0);
        super::types::is_i32_slot(kind)
    }

    /// Compile a binary operation using native Cranelift f64 instructions.
    ///
    /// NaN-boxed numbers are stored as plain f64 bits (not tagged), so we can
    /// bitcast i64→f64, do the operation natively, and bitcast f64→i64.
    /// Comparisons return NaN-boxed booleans (TAG_BOOL_TRUE / TAG_BOOL_FALSE).
    fn compile_binop(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        match op {
            // Arithmetic: bitcast to f64, operate, bitcast back
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                let l_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), lhs);
                let r_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), rhs);
                let result_f64 = match op {
                    BinOp::Add => self.builder.ins().fadd(l_f64, r_f64),
                    BinOp::Sub => self.builder.ins().fsub(l_f64, r_f64),
                    BinOp::Mul => self.builder.ins().fmul(l_f64, r_f64),
                    BinOp::Div => self.builder.ins().fdiv(l_f64, r_f64),
                    BinOp::Mod => {
                        // f64 modulo: a - floor(a/b) * b
                        let div = self.builder.ins().fdiv(l_f64, r_f64);
                        let floored = self.builder.ins().floor(div);
                        let prod = self.builder.ins().fmul(floored, r_f64);
                        self.builder.ins().fsub(l_f64, prod)
                    }
                    _ => unreachable!(),
                };
                Ok(self.builder.ins().bitcast(types::I64, MemFlags::new(), result_f64))
            }

            // Comparisons: bitcast to f64, compare, return NaN-boxed boolean
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let l_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), lhs);
                let r_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), rhs);
                let cc = match op {
                    BinOp::Eq => FloatCC::Equal,
                    BinOp::Ne => FloatCC::NotEqual,
                    BinOp::Lt => FloatCC::LessThan,
                    BinOp::Le => FloatCC::LessThanOrEqual,
                    BinOp::Gt => FloatCC::GreaterThan,
                    BinOp::Ge => FloatCC::GreaterThanOrEqual,
                    _ => unreachable!(),
                };
                let cmp = self.builder.ins().fcmp(cc, l_f64, r_f64);
                let true_val = self.builder.ins().iconst(
                    types::I64,
                    crate::nan_boxing::TAG_BOOL_TRUE as i64,
                );
                let false_val = self.builder.ins().iconst(
                    types::I64,
                    crate::nan_boxing::TAG_BOOL_FALSE as i64,
                );
                Ok(self.builder.ins().select(cmp, true_val, false_val))
            }

            // Logical: check truthiness (== TAG_BOOL_TRUE)
            BinOp::And => {
                let tag_true = self.builder.ins().iconst(
                    types::I64,
                    crate::nan_boxing::TAG_BOOL_TRUE as i64,
                );
                let l_is_true = self.builder.ins().icmp(IntCC::Equal, lhs, tag_true);
                let r_is_true = self.builder.ins().icmp(IntCC::Equal, rhs, tag_true);
                let both = self.builder.ins().band(l_is_true, r_is_true);
                let false_val = self.builder.ins().iconst(
                    types::I64,
                    crate::nan_boxing::TAG_BOOL_FALSE as i64,
                );
                Ok(self.builder.ins().select(both, tag_true, false_val))
            }
            BinOp::Or => {
                let tag_true = self.builder.ins().iconst(
                    types::I64,
                    crate::nan_boxing::TAG_BOOL_TRUE as i64,
                );
                let l_is_true = self.builder.ins().icmp(IntCC::Equal, lhs, tag_true);
                let r_is_true = self.builder.ins().icmp(IntCC::Equal, rhs, tag_true);
                let either = self.builder.ins().bor(l_is_true, r_is_true);
                let false_val = self.builder.ins().iconst(
                    types::I64,
                    crate::nan_boxing::TAG_BOOL_FALSE as i64,
                );
                Ok(self.builder.ins().select(either, tag_true, false_val))
            }
        }
    }

    /// Compile a unary operation.
    fn compile_unop(&mut self, op: &UnOp, val: Value) -> Result<Value, String> {
        match op {
            UnOp::Neg => {
                let f64_val = self.builder.ins().bitcast(types::F64, MemFlags::new(), val);
                let neg = self.builder.ins().fneg(f64_val);
                Ok(self.builder.ins().bitcast(types::I64, MemFlags::new(), neg))
            }
            UnOp::Not => {
                let tag_true = self.builder.ins().iconst(
                    types::I64,
                    crate::nan_boxing::TAG_BOOL_TRUE as i64,
                );
                let false_val = self.builder.ins().iconst(
                    types::I64,
                    crate::nan_boxing::TAG_BOOL_FALSE as i64,
                );
                let is_true = self.builder.ins().icmp(IntCC::Equal, val, tag_true);
                // !true = false, !false = true
                Ok(self.builder.ins().select(is_true, false_val, tag_true))
            }
        }
    }
}
