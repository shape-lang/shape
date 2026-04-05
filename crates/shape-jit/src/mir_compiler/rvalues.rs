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
                // Check source operand kinds BEFORE compiling (needed for I64 disambiguation).
                let lhs_kind = self.operand_slot_kind(lhs);
                let rhs_kind = self.operand_slot_kind(rhs);

                let l = self.compile_operand(lhs)?;
                let r = self.compile_operand(rhs)?;

                // Check operand types for native inline paths.
                let l_type = self.builder.func.dfg.value_type(l);
                let r_type = self.builder.func.dfg.value_type(r);

                if l_type == types::F64 && r_type == types::F64 {
                    // Both operands are native F64 — inline float ops.
                    self.compile_binop_f64(op, l, r)
                } else if l_type == types::I32 && r_type == types::I32 {
                    // Both operands are native I32 — inline i32 ops.
                    match op {
                        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                            self.compile_binop_i32_native(op, l, r)
                        }
                        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le
                        | BinOp::Gt | BinOp::Ge => {
                            self.compile_cmp_i32_native(op, l, r)
                        }
                        _ => self.compile_binop(op, l, r),
                    }
                } else if l_type == types::I8 && r_type == types::I8 {
                    // Both operands are native I8 (Bool) — inline bool ops.
                    self.compile_binop_bool(op, l, r)
                } else if self.both_int64(lhs_kind, rhs_kind) {
                    // Both operands are Int64 slots (NaN-boxed ints) — inline i64 arithmetic.
                    // Extract 48-bit payload, operate natively, re-box.
                    self.compile_binop_int64(op, l, r)
                } else {
                    // Mixed or unknown types — use FFI generic path.
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
                // v2-boundary: arc_retain FFI still takes NaN-boxed I64
                let boxed = self.ensure_nanboxed(val);
                self.builder.ins().call(self.ffi.arc_retain, &[boxed]);
                Ok(boxed)
            }

            Rvalue::Borrow(_kind, place) => {
                // StackSlot-based references (ported from BytecodeToIR):
                // 1. Read the current value from the place (box for stack slot)
                let raw_val = self.read_place(place)?;
                // v2-boundary: borrow stack slots store NaN-boxed I64
                let val = self.ensure_nanboxed(raw_val);
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
                let zero = self.builder.ins().iconst(types::I64, 0i64);
                let inst = self.builder.ins().call(
                    self.ffi.new_array,
                    &[self.ctx_ptr, zero],
                );
                let mut arr = self.builder.inst_results(inst)[0];

                // v2-boundary: jit_array_push_elem FFI still expects NaN-boxed I64 elements
                for operand in operands {
                    let raw = self.compile_operand_raw(operand)?;
                    let elem = self.ensure_nanboxed(raw);
                    let inst = self.builder
                        .ins()
                        .call(self.ffi.array_push_elem, &[arr, elem]);
                    arr = self.builder.inst_results(inst)[0];
                }

                Ok(arr)
            }
        }
    }

    // ── Operand kind helpers ───────────────────────────────────────

    /// Get the SlotKind of an operand's source (before compilation).
    fn operand_slot_kind(&self, operand: &Operand) -> Option<shape_vm::type_tracking::SlotKind> {
        match operand {
            Operand::Constant(MirConstant::Int(_)) => {
                Some(shape_vm::type_tracking::SlotKind::Int64)
            }
            Operand::Constant(MirConstant::Float(_)) => {
                Some(shape_vm::type_tracking::SlotKind::Float64)
            }
            Operand::Constant(MirConstant::Bool(_)) => {
                Some(shape_vm::type_tracking::SlotKind::Bool)
            }
            Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => {
                let slot = p.root_local();
                let kind = super::types::slot_kind_for_local(&self.slot_kinds, slot.0);
                if kind != shape_vm::type_tracking::SlotKind::Unknown {
                    Some(kind)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Check if both operand kinds are Int64 (NaN-boxed integers suitable for inline i64 ops).
    fn both_int64(
        &self,
        lhs: Option<shape_vm::type_tracking::SlotKind>,
        rhs: Option<shape_vm::type_tracking::SlotKind>,
    ) -> bool {
        matches!(
            (lhs, rhs),
            (
                Some(shape_vm::type_tracking::SlotKind::Int64),
                Some(shape_vm::type_tracking::SlotKind::Int64)
            )
        )
    }

    // ── Inline Float64 arithmetic and comparisons ──────────────────

    /// Compile a binary op on native F64 operands — direct Cranelift float instructions.
    /// ~100x faster per operation vs FFI generic_add/etc.
    fn compile_binop_f64(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        match op {
            BinOp::Add => Ok(self.builder.ins().fadd(lhs, rhs)),
            BinOp::Sub => Ok(self.builder.ins().fsub(lhs, rhs)),
            BinOp::Mul => Ok(self.builder.ins().fmul(lhs, rhs)),
            BinOp::Div => Ok(self.builder.ins().fdiv(lhs, rhs)),
            BinOp::Mod => {
                // f64 mod: a % b = a - trunc(a/b) * b (pure Cranelift, no FFI)
                let div = self.builder.ins().fdiv(lhs, rhs);
                let truncated = self.builder.ins().trunc(div);
                let product = self.builder.ins().fmul(truncated, rhs);
                Ok(self.builder.ins().fsub(lhs, product))
            }
            BinOp::Eq => {
                let cmp = self.builder.ins().fcmp(FloatCC::Equal, lhs, rhs);
                // fcmp returns I8 (native bool) — this is fine for Bool slots
                Ok(cmp)
            }
            BinOp::Ne => {
                let cmp = self.builder.ins().fcmp(FloatCC::NotEqual, lhs, rhs);
                Ok(cmp)
            }
            BinOp::Lt => {
                let cmp = self.builder.ins().fcmp(FloatCC::LessThan, lhs, rhs);
                Ok(cmp)
            }
            BinOp::Le => {
                let cmp = self.builder.ins().fcmp(FloatCC::LessThanOrEqual, lhs, rhs);
                Ok(cmp)
            }
            BinOp::Gt => {
                let cmp = self.builder.ins().fcmp(FloatCC::GreaterThan, lhs, rhs);
                Ok(cmp)
            }
            BinOp::Ge => {
                let cmp = self
                    .builder
                    .ins()
                    .fcmp(FloatCC::GreaterThanOrEqual, lhs, rhs);
                Ok(cmp)
            }
            BinOp::And | BinOp::Or => {
                // Logical ops on floats — box and use generic path
                self.compile_binop(op, lhs, rhs)
            }
        }
    }

    // ── Native I32 arithmetic (no ireduce/sextend needed) ───────────

    /// Compile i32 binary arithmetic on native I32 values (no boxing overhead).
    fn compile_binop_i32_native(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        match op {
            BinOp::Add => Ok(self.builder.ins().iadd(lhs, rhs)),
            BinOp::Sub => Ok(self.builder.ins().isub(lhs, rhs)),
            BinOp::Mul => Ok(self.builder.ins().imul(lhs, rhs)),
            BinOp::Div => {
                let zero = self.builder.ins().iconst(types::I32, 0);
                let is_zero = self.builder.ins().icmp(IntCC::Equal, rhs, zero);
                self.builder.ins().trapnz(is_zero, TrapCode::User(0));
                Ok(self.builder.ins().sdiv(lhs, rhs))
            }
            BinOp::Mod => {
                let zero = self.builder.ins().iconst(types::I32, 0);
                let is_zero = self.builder.ins().icmp(IntCC::Equal, rhs, zero);
                self.builder.ins().trapnz(is_zero, TrapCode::User(0));
                Ok(self.builder.ins().srem(lhs, rhs))
            }
            _ => Err(format!("unsupported native i32 binop: {:?}", op)),
        }
    }

    /// Compile i32 comparison on native I32 values — returns I8 (native bool).
    fn compile_cmp_i32_native(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        let cc = match op {
            BinOp::Eq => IntCC::Equal,
            BinOp::Ne => IntCC::NotEqual,
            BinOp::Lt => IntCC::SignedLessThan,
            BinOp::Le => IntCC::SignedLessThanOrEqual,
            BinOp::Gt => IntCC::SignedGreaterThan,
            BinOp::Ge => IntCC::SignedGreaterThanOrEqual,
            _ => return Err(format!("unsupported native i32 cmp: {:?}", op)),
        };
        // icmp returns I8 (native bool)
        Ok(self.builder.ins().icmp(cc, lhs, rhs))
    }

    // ── Inline Int64 arithmetic (NaN-boxed ints) ──────────────────

    /// Compile a binary op on proven Int64 operands — extract payload, operate, re-box.
    /// Eliminates FFI call overhead (~50-100ns → ~5ns per operation).
    fn compile_binop_int64(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        // Extract 48-bit signed int payload: shift left 16, arithmetic shift right 16
        let l = self.builder.ins().ishl_imm(lhs, 16);
        let l = self.builder.ins().sshr_imm(l, 16);
        let r = self.builder.ins().ishl_imm(rhs, 16);
        let r = self.builder.ins().sshr_imm(r, 16);

        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                let result = match op {
                    BinOp::Add => self.builder.ins().iadd(l, r),
                    BinOp::Sub => self.builder.ins().isub(l, r),
                    BinOp::Mul => self.builder.ins().imul(l, r),
                    BinOp::Div => {
                        let zero = self.builder.ins().iconst(types::I64, 0);
                        let is_zero = self.builder.ins().icmp(IntCC::Equal, r, zero);
                        self.builder.ins().trapnz(is_zero, TrapCode::User(0));
                        self.builder.ins().sdiv(l, r)
                    }
                    BinOp::Mod => {
                        let zero = self.builder.ins().iconst(types::I64, 0);
                        let is_zero = self.builder.ins().icmp(IntCC::Equal, r, zero);
                        self.builder.ins().trapnz(is_zero, TrapCode::User(0));
                        self.builder.ins().srem(l, r)
                    }
                    _ => unreachable!(),
                };
                // Re-box: mask to 48-bit payload, apply INT tag
                let payload_mask = self
                    .builder
                    .ins()
                    .iconst(types::I64, shape_value::tags::PAYLOAD_MASK as i64);
                let payload = self.builder.ins().band(result, payload_mask);
                let int_tag = self.builder.ins().iconst(
                    types::I64,
                    (shape_value::tags::TAG_BASE
                        | (shape_value::tags::TAG_INT << shape_value::tags::TAG_SHIFT))
                        as i64,
                );
                Ok(self.builder.ins().bor(int_tag, payload))
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let cc = match op {
                    BinOp::Eq => IntCC::Equal,
                    BinOp::Ne => IntCC::NotEqual,
                    BinOp::Lt => IntCC::SignedLessThan,
                    BinOp::Le => IntCC::SignedLessThanOrEqual,
                    BinOp::Gt => IntCC::SignedGreaterThan,
                    BinOp::Ge => IntCC::SignedGreaterThanOrEqual,
                    _ => unreachable!(),
                };
                let cmp = self.builder.ins().icmp(cc, l, r);
                // icmp returns I8 (native bool)
                Ok(cmp)
            }
            _ => {
                // Logical ops — use FFI path
                self.compile_binop(op, lhs, rhs)
            }
        }
    }

    // ── Native Bool operations ──────────────────────────────────────

    /// Compile a binary op on native I8 (Bool) operands.
    fn compile_binop_bool(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        match op {
            BinOp::Eq => Ok(self.builder.ins().icmp(IntCC::Equal, lhs, rhs)),
            BinOp::Ne => Ok(self.builder.ins().icmp(IntCC::NotEqual, lhs, rhs)),
            BinOp::And => Ok(self.builder.ins().band(lhs, rhs)),
            BinOp::Or => Ok(self.builder.ins().bor(lhs, rhs)),
            _ => {
                // Other ops on bools — box and use generic path
                self.compile_binop(op, lhs, rhs)
            }
        }
    }

    /// Compile a binary operation using generic FFI calls for unknown types.
    /// Ensures both operands are NaN-boxed I64 before calling FFI.
    fn compile_binop(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        // v2-boundary: generic_* FFI functions expect NaN-boxed I64 arguments
        let l = self.ensure_nanboxed(lhs);
        let r = self.ensure_nanboxed(rhs);
        match op {
            BinOp::Add => { let inst = self.builder.ins().call(self.ffi.generic_add, &[l, r]); Ok(self.builder.inst_results(inst)[0]) }
            BinOp::Sub => { let inst = self.builder.ins().call(self.ffi.generic_sub, &[l, r]); Ok(self.builder.inst_results(inst)[0]) }
            BinOp::Mul => { let inst = self.builder.ins().call(self.ffi.generic_mul, &[l, r]); Ok(self.builder.inst_results(inst)[0]) }
            BinOp::Div => { let inst = self.builder.ins().call(self.ffi.generic_div, &[l, r]); Ok(self.builder.inst_results(inst)[0]) }
            BinOp::Mod => { let inst = self.builder.ins().call(self.ffi.generic_mod, &[l, r]); Ok(self.builder.inst_results(inst)[0]) }
            BinOp::Eq => { let inst = self.builder.ins().call(self.ffi.generic_eq, &[l, r]); Ok(self.builder.inst_results(inst)[0]) }
            BinOp::Ne => { let inst = self.builder.ins().call(self.ffi.generic_neq, &[l, r]); Ok(self.builder.inst_results(inst)[0]) }
            BinOp::Lt => { let inst = self.builder.ins().call(self.ffi.generic_lt, &[l, r]); Ok(self.builder.inst_results(inst)[0]) }
            BinOp::Le => { let inst = self.builder.ins().call(self.ffi.generic_le, &[l, r]); Ok(self.builder.inst_results(inst)[0]) }
            BinOp::Gt => { let inst = self.builder.ins().call(self.ffi.generic_gt, &[l, r]); Ok(self.builder.inst_results(inst)[0]) }
            BinOp::Ge => { let inst = self.builder.ins().call(self.ffi.generic_ge, &[l, r]); Ok(self.builder.inst_results(inst)[0]) }

            // v2-boundary: logical ops on NaN-boxed values use TAG_BOOL_TRUE/FALSE
            BinOp::And => {
                let tag_true = self.builder.ins().iconst(
                    types::I64,
                    1i64,
                );
                let l_is_true = self.builder.ins().icmp(IntCC::Equal, l, tag_true);
                let r_is_true = self.builder.ins().icmp(IntCC::Equal, r, tag_true);
                let both = self.builder.ins().band(l_is_true, r_is_true);
                let false_val = self.builder.ins().iconst(
                    types::I64,
                    0i64,
                );
                Ok(self.builder.ins().select(both, tag_true, false_val))
            }
            BinOp::Or => {
                let tag_true = self.builder.ins().iconst(
                    types::I64,
                    1i64,
                );
                let l_is_true = self.builder.ins().icmp(IntCC::Equal, l, tag_true);
                let r_is_true = self.builder.ins().icmp(IntCC::Equal, r, tag_true);
                let either = self.builder.ins().bor(l_is_true, r_is_true);
                let false_val = self.builder.ins().iconst(
                    types::I64,
                    0i64,
                );
                Ok(self.builder.ins().select(either, tag_true, false_val))
            }
        }
    }

    /// Compile a unary operation.
    fn compile_unop(&mut self, op: &UnOp, val: Value) -> Result<Value, String> {
        let val_type = self.builder.func.dfg.value_type(val);
        match op {
            UnOp::Neg => {
                if val_type == types::F64 {
                    // Native F64: direct fneg
                    Ok(self.builder.ins().fneg(val))
                } else {
                    // NaN-boxed: bitcast to F64, negate, bitcast back
                    let f64_val = self.builder.ins().bitcast(types::F64, MemFlags::new(), val);
                    let neg = self.builder.ins().fneg(f64_val);
                    Ok(self.builder.ins().bitcast(types::I64, MemFlags::new(), neg))
                }
            }
            UnOp::Not => {
                if val_type == types::I8 {
                    // Native I8 bool: XOR with 1 to flip
                    let one = self.builder.ins().iconst(types::I8, 1);
                    Ok(self.builder.ins().bxor(val, one))
                } else {
                    // v2-boundary: NaN-boxed bool uses TAG_BOOL_TRUE/FALSE tags
                    let tag_true = self.builder.ins().iconst(
                        types::I64,
                        1i64,
                    );
                    let false_val = self.builder.ins().iconst(
                        types::I64,
                        0i64,
                    );
                    let is_true = self.builder.ins().icmp(IntCC::Equal, val, tag_true);
                    Ok(self.builder.ins().select(is_true, false_val, tag_true))
                }
            }
        }
    }
}
