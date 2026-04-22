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

                // F5.a/F5.b: string `+` — concat via FFI. Either operand being a
                // `SlotKind::String` is enough; the FFI handles `str + <any>` by
                // falling back to `format_value_word` on non-string operands,
                // which matches the lowering emitted by f-string interpolation.
                if matches!(op, BinOp::Add) && self.either_string(lhs_kind, rhs_kind) {
                    return self.compile_string_concat(l, r);
                }

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
                //
                // R4.2D: `jit_arc_retain` takes a plain `u64` bit-pattern
                // (implicitly ValueWord-encoded), so no width-extension
                // wrap is needed. Clones are only emitted for heap types,
                // which already live in I64 slots at this site.
                let val = self.compile_operand_raw(operand)?;
                self.builder.ins().call(self.ffi.arc_retain, &[val]);
                Ok(val)
            }

            Rvalue::Borrow(_kind, place) => {
                // R4.2F: allocate a native-sized/aligned stack cell that
                // matches the root local's Cranelift type. References are
                // strictly per-function — they never cross Cranelift call
                // boundaries — so picking a native width here is safe and
                // removes the width-extension wrap/unwrap pair.
                //
                // For non-native slot kinds (heap / string / unknown),
                // `cranelift_type_for_slot` returns I64, collapsing to the
                // legacy 8-byte cell with no behavioural change.
                let raw_val = self.read_place(place)?;
                let root = place.root_local();
                let kind = super::types::slot_kind_for_local(&self.slot_kinds, root.0);
                let cl_ty = super::types::cranelift_type_for_slot(kind);
                let size = cl_ty.bytes();
                // `create_sized_stack_slot` takes the log2 of the alignment;
                // `trailing_zeros` of a power-of-two size is exactly that.
                let align_shift = size.trailing_zeros() as u8;
                let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    size,
                    align_shift,
                ));
                // Store the value at its native width — no NaN-box wrap.
                self.builder.ins().stack_store(raw_val, slot, 0);
                // Track root local + native type for reload-after-call.
                self.ref_stack_slots.insert(root, (slot, cl_ty));
                // Return the stack slot address as the reference value.
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

                // R4.2B: FFI signatures accept plain u64 bit-patterns — no
                // box wrap needed at call site. Operands reaching
                // `jit_array_push_elem` are ValueWord-encoded I64 slots. Native
                // F64/I32/I8 constants flowing in from `compile_operand_raw`
                // must be widened/bitcast to I64 first so the Cranelift
                // verifier accepts the call's `i64` parameter types.
                for operand in operands {
                    let elem_raw = self.compile_operand_raw(operand)?;
                    let elem = self.widen_to_i64(elem_raw);
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

    /// F5.a/F5.b: true if either operand kind is `SlotKind::String`. The MIR
    /// emits `BinOp::Add` on heterogeneous operand types for f-string
    /// interpolation (e.g. `str + number + str`) — the FFI's non-string
    /// fallback (`format_value_word`) does the rest.
    fn either_string(
        &self,
        lhs: Option<shape_vm::type_tracking::SlotKind>,
        rhs: Option<shape_vm::type_tracking::SlotKind>,
    ) -> bool {
        matches!(lhs, Some(shape_vm::type_tracking::SlotKind::String))
            || matches!(rhs, Some(shape_vm::type_tracking::SlotKind::String))
    }

    /// F5.a/F5.b: emit a call to `jit_string_concat(a_bits, b_bits) -> bits`.
    ///
    /// Both operand `Value`s must be widened to I64 bit-patterns (the FFI
    /// signature expects two `i64` params). This handles the cases where the
    /// MIR lowering produced a native-typed constant for one side — e.g.
    /// `f"x={n}"` where `n: int` is `SlotKind::Int64` (I64 bits already) or
    /// a plain number constant (F64, must bitcast to I64).
    fn compile_string_concat(
        &mut self,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        let a = self.to_i64_bits(lhs);
        let b = self.to_i64_bits(rhs);
        let inst = self.builder.ins().call(self.ffi.string_concat, &[a, b]);
        Ok(self.builder.inst_results(inst)[0])
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
                    .iconst(types::I64, shape_value::tag_bits::PAYLOAD_MASK as i64);
                let payload = self.builder.ins().band(result, payload_mask);
                let int_tag = self.builder.ins().iconst(
                    types::I64,
                    (shape_value::tag_bits::TAG_BASE
                        | (shape_value::tag_bits::TAG_INT << shape_value::tag_bits::TAG_SHIFT))
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

    /// Compile a binary operation on a dynamic (NaN-boxed) slot.
    ///
    /// R7.1: After R5.1–R5.6 retargeted all dynamic arithmetic /
    /// comparison fallbacks (typed bitwise, user operator traits,
    /// DateTime, Matrix/Vec, string+scalar) to typed opcodes or
    /// `CallMethod`, the JIT no longer receives fully dynamic
    /// arithmetic / comparison binops from MIR. The `generic_*`
    /// FFI trampolines (`generic_add`/`sub`/`mul`/`div`/`mod`,
    /// `generic_eq`/`neq`, `generic_lt`/`le`/`gt`/`ge`) were the
    /// last things pinning those FuncRefs alive and have been
    /// removed in this commit.
    ///
    /// This helper remains for the `BinOp::And` / `BinOp::Or`
    /// fallthroughs from `compile_binop_f64`, `compile_binop_int64`,
    /// and `compile_binop_bool` where the logical op mixes with a
    /// NaN-boxed bool encoding (TAG_BOOL_TRUE / TAG_BOOL_FALSE).
    ///
    /// Session 2: Dynamic arithmetic binops from CallValue-returned
    /// slots (closure calls whose return type isn't provable at MIR
    /// level) are lowered via an inline NaN-box dispatch — `Both-Number`
    /// (hot path: `!is_tagged(l) && !is_tagged(r)` → native fadd/etc.) or
    /// `Both-Int` (`is_tagged_int(l) && is_tagged_int(r)` → i48 math).
    /// Mixed or heap operands trap the JIT function, triggering an
    /// error-signal return that the caller observes via the deopt
    /// pathway. This preserves `no generic_* FFI` while keeping
    /// closure-return-arith JIT-compilable.
    fn compile_binop(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        // Widen native-typed operands into their NaN-boxed I64 bit-pattern so
        // the dynamic dispatch helpers can treat both uniformly. This handles
        // the mixed cases (e.g. F64 literal vs I64 NaN-boxed heap handle)
        // that `compile_rvalue` routes here after the typed fast paths.
        let l = self.to_i64_bits(lhs);
        let r = self.to_i64_bits(rhs);
        match op {
            BinOp::Add
            | BinOp::Sub
            | BinOp::Mul
            | BinOp::Div
            | BinOp::Mod => self.compile_binop_dynamic_arith(op, l, r),

            BinOp::Eq
            | BinOp::Ne
            | BinOp::Lt
            | BinOp::Le
            | BinOp::Gt
            | BinOp::Ge => self.compile_binop_dynamic_cmp(op, l, r),

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

    // ── Session 2: Dynamic arith / cmp inline NaN-box dispatch ────────

    /// Widen an operand Value to its NaN-boxed I64 bit-pattern.
    ///
    /// - `F64` → bitcast to `I64` (the f64 bit-pattern *is* the NaN-box payload
    ///   because plain numbers have sign=0).
    /// - `I32` / `I16` → sign-extend to `I64`. NaN-boxed int slots use
    ///   `TAG_INT | (i48_payload_mask & value)` upstream; narrow-int slots
    ///   reaching `compile_binop` are rare (the native-I32 fast path catches
    ///   both-I32 already), so this conservative sign-extend keeps the raw
    ///   integer value visible to the dynamic dispatch's `int` branch.
    /// - `I8` (native bool) → zero-extend to `I64`. The logical-op branches of
    ///   `compile_binop` compare against the literal `1i64` ⇔ `TAG_BOOL_TRUE`
    ///   encoding, so widening to I64 preserves truth semantics.
    /// - `I64` → passed through unchanged.
    fn to_i64_bits(&mut self, v: Value) -> Value {
        let ty = self.builder.func.dfg.value_type(v);
        if ty == types::I64 {
            v
        } else if ty == types::F64 {
            self.builder.ins().bitcast(types::I64, MemFlags::new(), v)
        } else if ty == types::I32 || ty == types::I16 {
            self.builder.ins().sextend(types::I64, v)
        } else if ty == types::I8 {
            self.builder.ins().uextend(types::I64, v)
        } else {
            v
        }
    }


    /// Compile a dynamic-operand arithmetic binop (Add/Sub/Mul/Div/Mod).
    ///
    /// Both operands arrive as NaN-boxed `I64` bit-patterns. The emitted
    /// IR branches on the tag bits:
    ///
    /// - Both numbers (`sign==0` ⇒ `!is_tagged`): `bitcast→fadd→bitcast`
    ///   to stay in the f64 domain.
    /// - Both i48-tagged ints: sign-extend 48-bit payload, native i64 op,
    ///   re-box with `TAG_INT`.
    /// - Otherwise: trap (caller's deopt path converts to an error).
    ///
    /// This is the JIT analogue of the VM's `AddDynamic` IC fast path —
    /// in practice the operands come from closure calls whose return
    /// value the MIR couldn't type-prove, and are always `Number` or
    /// `Int` at runtime for the tests in the closure test set.
    fn compile_binop_dynamic_arith(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        // Tagged-test masks.
        let tag_base = self.builder.ins().iconst(
            types::I64,
            shape_value::tag_bits::TAG_BASE as i64,
        );
        let l_masked = self.builder.ins().band(lhs, tag_base);
        let r_masked = self.builder.ins().band(rhs, tag_base);

        // Is each operand a plain f64 (sign bit zero ⇒ NaN-box prefix absent)?
        let l_is_num = self.builder.ins().icmp(IntCC::NotEqual, l_masked, tag_base);
        let r_is_num = self.builder.ins().icmp(IntCC::NotEqual, r_masked, tag_base);
        let both_num = self.builder.ins().band(l_is_num, r_is_num);

        // Is each operand a TAG_INT tagged value?
        let int_prefix = self.builder.ins().iconst(
            types::I64,
            (shape_value::tag_bits::TAG_BASE
                | (shape_value::tag_bits::TAG_INT << shape_value::tag_bits::TAG_SHIFT))
                as i64,
        );
        let tag_mask_full = self.builder.ins().iconst(
            types::I64,
            (shape_value::tag_bits::TAG_BASE | shape_value::tag_bits::TAG_MASK) as i64,
        );
        let l_tag_only = self.builder.ins().band(lhs, tag_mask_full);
        let r_tag_only = self.builder.ins().band(rhs, tag_mask_full);
        let l_is_int = self.builder.ins().icmp(IntCC::Equal, l_tag_only, int_prefix);
        let r_is_int = self.builder.ins().icmp(IntCC::Equal, r_tag_only, int_prefix);
        let both_int = self.builder.ins().band(l_is_int, r_is_int);

        // Block layout:
        //   current    -> brif both_num, num_block, maybe_int_block
        //   num_block  -> fadd etc.; jump merge(f_result_as_i64)
        //   maybe_int_block -> brif both_int, int_block, trap_block
        //   int_block  -> i48 math; jump merge(int_result)
        //   trap_block -> trap
        //   merge      -> block-param result
        let num_block = self.builder.create_block();
        let maybe_int_block = self.builder.create_block();
        let int_block = self.builder.create_block();
        let trap_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I64);

        self.builder
            .ins()
            .brif(both_num, num_block, &[], maybe_int_block, &[]);

        // ── Both-number path ──────────────────────────────────────────
        self.builder.switch_to_block(num_block);
        self.builder.seal_block(num_block);
        let lf = self.builder.ins().bitcast(types::F64, MemFlags::new(), lhs);
        let rf = self.builder.ins().bitcast(types::F64, MemFlags::new(), rhs);
        let nres = match op {
            BinOp::Add => self.builder.ins().fadd(lf, rf),
            BinOp::Sub => self.builder.ins().fsub(lf, rf),
            BinOp::Mul => self.builder.ins().fmul(lf, rf),
            BinOp::Div => self.builder.ins().fdiv(lf, rf),
            BinOp::Mod => {
                let div = self.builder.ins().fdiv(lf, rf);
                let truncated = self.builder.ins().trunc(div);
                let product = self.builder.ins().fmul(truncated, rf);
                self.builder.ins().fsub(lf, product)
            }
            _ => unreachable!("compile_binop_dynamic_arith: non-arith op {:?}", op),
        };
        let nres_bits = self.builder.ins().bitcast(types::I64, MemFlags::new(), nres);
        self.builder.ins().jump(merge_block, &[nres_bits]);

        // ── Both-int path ─────────────────────────────────────────────
        self.builder.switch_to_block(maybe_int_block);
        self.builder.seal_block(maybe_int_block);
        self.builder
            .ins()
            .brif(both_int, int_block, &[], trap_block, &[]);

        self.builder.switch_to_block(int_block);
        self.builder.seal_block(int_block);
        // Extract 48-bit signed int payload: shift left 16, asr 16.
        let li = self.builder.ins().ishl_imm(lhs, 16);
        let li = self.builder.ins().sshr_imm(li, 16);
        let ri = self.builder.ins().ishl_imm(rhs, 16);
        let ri = self.builder.ins().sshr_imm(ri, 16);
        let ires = match op {
            BinOp::Add => self.builder.ins().iadd(li, ri),
            BinOp::Sub => self.builder.ins().isub(li, ri),
            BinOp::Mul => self.builder.ins().imul(li, ri),
            BinOp::Div => {
                let zero = self.builder.ins().iconst(types::I64, 0);
                let is_zero = self.builder.ins().icmp(IntCC::Equal, ri, zero);
                self.builder.ins().trapnz(is_zero, TrapCode::User(0));
                self.builder.ins().sdiv(li, ri)
            }
            BinOp::Mod => {
                let zero = self.builder.ins().iconst(types::I64, 0);
                let is_zero = self.builder.ins().icmp(IntCC::Equal, ri, zero);
                self.builder.ins().trapnz(is_zero, TrapCode::User(0));
                self.builder.ins().srem(li, ri)
            }
            _ => unreachable!("compile_binop_dynamic_arith: non-arith op {:?}", op),
        };
        let payload_mask = self.builder.ins().iconst(
            types::I64,
            shape_value::tag_bits::PAYLOAD_MASK as i64,
        );
        let ipayload = self.builder.ins().band(ires, payload_mask);
        let iboxed = self.builder.ins().bor(int_prefix, ipayload);
        self.builder.ins().jump(merge_block, &[iboxed]);

        // ── Trap path ─────────────────────────────────────────────────
        // Emit a negative error signal return so the caller observes a JIT
        // deopt rather than an illegal-instruction trap. This matches the
        // error-signal convention used by direct-call terminators.
        self.builder.switch_to_block(trap_block);
        self.builder.seal_block(trap_block);
        let signal = self.builder.ins().iconst(types::I32, 0xFFFF_FFFFu64 as i64);
        self.builder.ins().return_(&[signal]);

        // ── Merge ─────────────────────────────────────────────────────
        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        Ok(self.builder.block_params(merge_block)[0])
    }

    /// Compile a dynamic-operand comparison binop (Eq/Ne/Lt/Le/Gt/Ge).
    ///
    /// Branches by tag exactly like `compile_binop_dynamic_arith`, with
    /// result type `I8` (native bool). Eq/Ne on mixed-tag operands are
    /// routed through the `icmp.Equal` on the raw bits — two values with
    /// different tags are never equal so the bitwise compare is correct.
    /// Lt/Le/Gt/Ge on mixed-tag operands trap.
    fn compile_binop_dynamic_cmp(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        // Bitwise Eq/Ne: any mismatched tag also means values are not equal.
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            let cc = if matches!(op, BinOp::Eq) {
                IntCC::Equal
            } else {
                IntCC::NotEqual
            };
            return Ok(self.builder.ins().icmp(cc, lhs, rhs));
        }

        // Tagged-test masks (same as arith).
        let tag_base = self.builder.ins().iconst(
            types::I64,
            shape_value::tag_bits::TAG_BASE as i64,
        );
        let l_masked = self.builder.ins().band(lhs, tag_base);
        let r_masked = self.builder.ins().band(rhs, tag_base);
        let l_is_num = self.builder.ins().icmp(IntCC::NotEqual, l_masked, tag_base);
        let r_is_num = self.builder.ins().icmp(IntCC::NotEqual, r_masked, tag_base);
        let both_num = self.builder.ins().band(l_is_num, r_is_num);

        let int_prefix = self.builder.ins().iconst(
            types::I64,
            (shape_value::tag_bits::TAG_BASE
                | (shape_value::tag_bits::TAG_INT << shape_value::tag_bits::TAG_SHIFT))
                as i64,
        );
        let tag_mask_full = self.builder.ins().iconst(
            types::I64,
            (shape_value::tag_bits::TAG_BASE | shape_value::tag_bits::TAG_MASK) as i64,
        );
        let l_tag_only = self.builder.ins().band(lhs, tag_mask_full);
        let r_tag_only = self.builder.ins().band(rhs, tag_mask_full);
        let l_is_int = self.builder.ins().icmp(IntCC::Equal, l_tag_only, int_prefix);
        let r_is_int = self.builder.ins().icmp(IntCC::Equal, r_tag_only, int_prefix);
        let both_int = self.builder.ins().band(l_is_int, r_is_int);

        let num_block = self.builder.create_block();
        let maybe_int_block = self.builder.create_block();
        let int_block = self.builder.create_block();
        let trap_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I8);

        self.builder
            .ins()
            .brif(both_num, num_block, &[], maybe_int_block, &[]);

        // Both-number path.
        self.builder.switch_to_block(num_block);
        self.builder.seal_block(num_block);
        let lf = self.builder.ins().bitcast(types::F64, MemFlags::new(), lhs);
        let rf = self.builder.ins().bitcast(types::F64, MemFlags::new(), rhs);
        let fcc = match op {
            BinOp::Lt => FloatCC::LessThan,
            BinOp::Le => FloatCC::LessThanOrEqual,
            BinOp::Gt => FloatCC::GreaterThan,
            BinOp::Ge => FloatCC::GreaterThanOrEqual,
            _ => unreachable!("compile_binop_dynamic_cmp: non-cmp op {:?}", op),
        };
        let ncmp = self.builder.ins().fcmp(fcc, lf, rf);
        self.builder.ins().jump(merge_block, &[ncmp]);

        // Both-int path.
        self.builder.switch_to_block(maybe_int_block);
        self.builder.seal_block(maybe_int_block);
        self.builder
            .ins()
            .brif(both_int, int_block, &[], trap_block, &[]);

        self.builder.switch_to_block(int_block);
        self.builder.seal_block(int_block);
        let li = self.builder.ins().ishl_imm(lhs, 16);
        let li = self.builder.ins().sshr_imm(li, 16);
        let ri = self.builder.ins().ishl_imm(rhs, 16);
        let ri = self.builder.ins().sshr_imm(ri, 16);
        let icc = match op {
            BinOp::Lt => IntCC::SignedLessThan,
            BinOp::Le => IntCC::SignedLessThanOrEqual,
            BinOp::Gt => IntCC::SignedGreaterThan,
            BinOp::Ge => IntCC::SignedGreaterThanOrEqual,
            _ => unreachable!("compile_binop_dynamic_cmp: non-cmp op {:?}", op),
        };
        let icmp = self.builder.ins().icmp(icc, li, ri);
        self.builder.ins().jump(merge_block, &[icmp]);

        // Trap path: emit a negative error signal return (same convention as
        // the arith helper) so deopt is observed as a JIT compile/run error
        // rather than an illegal instruction.
        self.builder.switch_to_block(trap_block);
        self.builder.seal_block(trap_block);
        let signal = self.builder.ins().iconst(types::I32, 0xFFFF_FFFFu64 as i64);
        self.builder.ins().return_(&[signal]);

        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        Ok(self.builder.block_params(merge_block)[0])
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
