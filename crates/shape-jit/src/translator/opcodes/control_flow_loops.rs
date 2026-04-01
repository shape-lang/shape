//! Loop control flow lowering: loop start/end, break/continue.

use std::collections::HashSet;

use cranelift::prelude::*;
use shape_vm::bytecode::{OpCode, Operand};

use crate::optimizer::{AffineGuardArraySource, LinearBoundGuard};
use crate::translator::types::{BytecodeToIR, LoopContext};

fn is_numeric_arith(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Div
            | OpCode::Mod
            | OpCode::Pow
            | OpCode::AddInt
            | OpCode::SubInt
            | OpCode::MulInt
            | OpCode::DivInt
            | OpCode::ModInt
            | OpCode::PowInt
            | OpCode::AddNumber
            | OpCode::SubNumber
            | OpCode::MulNumber
            | OpCode::DivNumber
            | OpCode::ModNumber
            | OpCode::PowNumber
    )
}

fn is_affine_unroll_safe_opcode(op: OpCode) -> bool {
    matches!(
        op,
        // Stack/value movement
        OpCode::PushConst
            | OpCode::PushNull
            | OpCode::Pop
            | OpCode::Dup
            | OpCode::Swap
            // Local/module binding access
            | OpCode::LoadLocal
            | OpCode::StoreLocal
            | OpCode::LoadModuleBinding
            | OpCode::StoreModuleBinding
            // Numeric ops
            | OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Div
            | OpCode::Mod
            | OpCode::Pow
            | OpCode::AddInt
            | OpCode::SubInt
            | OpCode::MulInt
            | OpCode::DivInt
            | OpCode::ModInt
            | OpCode::AddNumber
            | OpCode::SubNumber
            | OpCode::MulNumber
            | OpCode::DivNumber
            | OpCode::ModNumber
            // Comparisons/branches for loop control
            | OpCode::Eq
            | OpCode::Neq
            | OpCode::Gt
            | OpCode::Lt
            | OpCode::Gte
            | OpCode::Lte
            | OpCode::GtInt
            | OpCode::LtInt
            | OpCode::GteInt
            | OpCode::LteInt
            | OpCode::EqInt
            | OpCode::NeqInt
            | OpCode::GtNumber
            | OpCode::LtNumber
            | OpCode::GteNumber
            | OpCode::LteNumber
            | OpCode::EqNumber
            | OpCode::NeqNumber
            | OpCode::Jump
            | OpCode::JumpIfFalse
            | OpCode::JumpIfFalseTrusted
            | OpCode::JumpIfTrue
            | OpCode::Break
            | OpCode::Continue
            // Numeric array/reference access
            | OpCode::GetProp
            | OpCode::SetLocalIndex
            | OpCode::SetModuleBindingIndex
            | OpCode::SetIndexRef
            | OpCode::ArrayPushLocal
            | OpCode::MakeRef
            | OpCode::DerefLoad
            | OpCode::DerefStore
            | OpCode::Length
            // Drop ops are side-effect free in JIT lowering.
            | OpCode::DropCall
            | OpCode::DropCallAsync
            // Scalar coercions
            | OpCode::IntToNumber
            | OpCode::NumberToInt
    )
}

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    fn loop_body_is_affine_unroll_safe(
        &self,
        info: &crate::translator::loop_analysis::LoopInfo,
    ) -> bool {
        ((info.header_idx + 1)..info.end_idx)
            .all(|i| is_affine_unroll_safe_opcode(self.program.instructions[i].opcode))
    }

    /// Check if a local feeds IntToNumber inside the loop body.
    /// Scans for `LoadLocal(idx) IntToNumber` or `LoadLocalTrusted(idx) IntToNumber` patterns.
    fn local_feeds_int_to_number(
        instructions: &[shape_vm::bytecode::Instruction],
        info: &crate::translator::loop_analysis::LoopInfo,
        local_idx: u16,
    ) -> bool {
        use shape_vm::bytecode::{OpCode, Operand};

        for i in (info.header_idx + 1)..info.end_idx.saturating_sub(1) {
            let instr = &instructions[i];
            if matches!(instr.opcode, OpCode::LoadLocal | OpCode::LoadLocalTrusted) {
                if let Some(Operand::Local(idx)) = &instr.operand {
                    if *idx == local_idx {
                        let next = &instructions[i + 1];
                        if next.opcode == OpCode::IntToNumber {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn is_affine_numeric_unroll_candidate(
        &self,
        info: &crate::translator::loop_analysis::LoopInfo,
    ) -> bool {
        let Some(loop_plan) = self.optimization_plan.loops.get(&info.header_idx) else {
            return false;
        };
        if loop_plan.canonical_iv.is_none()
            || loop_plan.bound_slot.is_none()
            || loop_plan.step_value != Some(1)
        {
            return false;
        }
        if !self.loop_body_is_affine_unroll_safe(info) {
            return false;
        }

        let mut numeric_ops = 0usize;
        let mut affine_memory_ops = 0usize;
        for i in (info.header_idx + 1)..info.end_idx {
            let op = self.program.instructions[i].opcode;
            if is_numeric_arith(op) {
                numeric_ops += 1;
            }
            if self
                .optimization_plan
                .trusted_array_get_indices
                .contains(&i)
                || self
                    .optimization_plan
                    .trusted_array_set_indices
                    .contains(&i)
                || self
                    .optimization_plan
                    .non_negative_array_get_indices
                    .contains(&i)
                || matches!(
                    op,
                    OpCode::GetProp | OpCode::SetLocalIndex | OpCode::SetModuleBindingIndex
                )
            {
                affine_memory_ops += 1;
            }
        }

        numeric_ops >= 2 && affine_memory_ops >= 1
    }

    fn read_local_as_i64(&mut self, local_slot: u16) -> Value {
        if self.unboxed_int_locals.contains(&local_slot) {
            let var = self.get_or_create_local(local_slot);
            return self.builder.use_var(var);
        }

        if self.unboxed_f64_locals.contains(&local_slot) {
            if let Some(&f64_var) = self.f64_local_vars.get(&local_slot) {
                let f64_val = self.builder.use_var(f64_var);
                return self.builder.ins().fcvt_to_sint_sat(types::I64, f64_val);
            }
        }

        let var = self.get_or_create_local(local_slot);
        let boxed = self.builder.use_var(var);
        let as_f64 = self.i64_to_f64(boxed);
        self.builder.ins().fcvt_to_sint_sat(types::I64, as_f64)
    }

    fn read_module_binding_boxed(&mut self, mb_slot: u16) -> Value {
        if let Some(&var) = self.promoted_module_bindings.get(&mb_slot) {
            return self.builder.use_var(var);
        }
        let byte_offset = crate::context::LOCALS_OFFSET + (mb_slot as i32 * 8);
        self.builder
            .ins()
            .load(types::I64, MemFlags::new(), self.ctx_ptr, byte_offset)
    }

    fn read_array_length_for_guard(&mut self, source: AffineGuardArraySource) -> Value {
        match source {
            AffineGuardArraySource::Local(local_slot) => {
                if let Some((_, len)) = self.hoisted_array_info.get(&local_slot).copied() {
                    len
                } else {
                    let var = self.get_or_create_local(local_slot);
                    let arr_boxed = self.builder.use_var(var);
                    let (_, len) = self.emit_array_data_ptr(arr_boxed);
                    len
                }
            }
            AffineGuardArraySource::RefLocal(ref_slot) => {
                if let Some((_, len)) = self.hoisted_ref_array_info.get(&ref_slot).copied() {
                    len
                } else {
                    let var = self.get_or_create_local(ref_slot);
                    let ref_addr = self.builder.use_var(var);
                    let arr_boxed =
                        self.builder
                            .ins()
                            .load(types::I64, MemFlags::new(), ref_addr, 0);
                    let (_, len) = self.emit_array_data_ptr(arr_boxed);
                    len
                }
            }
            AffineGuardArraySource::ModuleBinding(binding) => {
                let arr_boxed = self.read_module_binding_boxed(binding);
                let (_, len) = self.emit_array_data_ptr(arr_boxed);
                len
            }
        }
    }

    fn emit_non_negative_step_guards(&mut self, loop_header: usize) {
        let Some(step_slots) = self
            .optimization_plan
            .non_negative_step_guards_by_loop
            .get(&loop_header)
        else {
            return;
        };
        let step_slots = step_slots.clone();
        for step_slot in step_slots {
            let step_i64 = self.read_local_as_i64(step_slot);
            let zero_i64 = self.builder.ins().iconst(types::I64, 0);
            let non_negative =
                self.builder
                    .ins()
                    .icmp(IntCC::SignedGreaterThanOrEqual, step_i64, zero_i64);
            let one = self.builder.ins().iconst(types::I8, 1);
            let zero = self.builder.ins().iconst(types::I8, 0);
            let non_negative_i8 = self.builder.ins().select(non_negative, one, zero);
            self.builder.ins().trapz(non_negative_i8, TrapCode::User(0));
        }
    }

    fn emit_non_negative_iv_guards(&mut self, loop_header: usize) {
        let Some(iv_slots) = self
            .optimization_plan
            .non_negative_iv_guards_by_loop
            .get(&loop_header)
        else {
            return;
        };
        let iv_slots = iv_slots.clone();
        for iv_slot in iv_slots {
            let iv_i64 = self.read_local_as_i64(iv_slot);
            let zero_i64 = self.builder.ins().iconst(types::I64, 0);
            let non_negative =
                self.builder
                    .ins()
                    .icmp(IntCC::SignedGreaterThanOrEqual, iv_i64, zero_i64);
            let one = self.builder.ins().iconst(types::I8, 1);
            let zero = self.builder.ins().iconst(types::I8, 0);
            let non_negative_i8 = self.builder.ins().select(non_negative, one, zero);
            self.builder.ins().trapz(non_negative_i8, TrapCode::User(0));
        }
    }

    fn local_initialized_as_array_before_loop(&self, loop_header: usize, local_slot: u16) -> bool {
        if loop_header == 0 {
            return false;
        }
        for i in (1..loop_header).rev() {
            let store = &self.program.instructions[i];
            if !matches!(
                (store.opcode, store.operand.as_ref()),
                (OpCode::StoreLocal, Some(Operand::Local(slot))) if *slot == local_slot
            ) {
                continue;
            }
            let prev = &self.program.instructions[i - 1];
            return prev.opcode == OpCode::NewArray;
        }
        false
    }

    fn local_initialized_as_int_before_loop(
        &self,
        loop_header: usize,
        local_slot: u16,
        expected: i64,
    ) -> bool {
        use shape_vm::bytecode::Constant;

        if loop_header == 0 {
            return false;
        }
        for i in (1..loop_header).rev() {
            let store = &self.program.instructions[i];
            if !matches!(
                (store.opcode, store.operand.as_ref()),
                (OpCode::StoreLocal, Some(Operand::Local(slot))) if *slot == local_slot
            ) {
                continue;
            }
            let prev = &self.program.instructions[i - 1];
            let init_matches = matches!(
                (prev.opcode, prev.operand.as_ref()),
                (OpCode::PushConst, Some(Operand::Const(const_idx)))
                    if matches!(
                        self.program.constants.get(*const_idx as usize),
                        Some(Constant::Int(v)) if *v == expected
                    )
            );
            if !init_matches {
                return false;
            }
            let rewritten = ((i + 1)..loop_header).any(|j| {
                matches!(
                    (&self.program.instructions[j].opcode, self.program.instructions[j].operand.as_ref()),
                    (OpCode::StoreLocal, Some(Operand::Local(slot))) if *slot == local_slot
                )
            });
            return !rewritten;
        }
        false
    }

    fn boxed_bits_for_fill_const(&self, const_idx: u16) -> Option<u64> {
        use shape_vm::bytecode::Constant;

        match self.program.constants.get(const_idx as usize) {
            Some(Constant::Int(v)) => Some(crate::nan_boxing::box_number(*v as f64)),
            Some(Constant::UInt(v)) => {
                // Only box as f64 if the value fits precisely; large u64 needs heap
                if *v <= (1u64 << 53) {
                    Some(crate::nan_boxing::box_number(*v as f64))
                } else {
                    None // Can't represent large u64 as inline NaN-boxed f64
                }
            }
            Some(Constant::Number(v)) => Some(crate::nan_boxing::box_number(*v)),
            Some(Constant::Bool(true)) => Some(crate::nan_boxing::TAG_BOOL_TRUE),
            Some(Constant::Bool(false)) => Some(crate::nan_boxing::TAG_BOOL_FALSE),
            Some(Constant::Null) | Some(Constant::Unit) => Some(crate::nan_boxing::TAG_NULL),
            _ => None,
        }
    }

    fn detect_fill_push_value_bits(
        &self,
        info: &crate::translator::loop_analysis::LoopInfo,
        iv_slot: u16,
        bound_slot: u16,
        push_site_idx: usize,
        push_local: u16,
    ) -> Option<u64> {
        let value_bits = if push_site_idx > 0 {
            let prev = &self.program.instructions[push_site_idx - 1];
            match (prev.opcode, prev.operand.as_ref()) {
                (OpCode::PushConst, Some(Operand::Const(const_idx))) => {
                    self.boxed_bits_for_fill_const(*const_idx)?
                }
                (OpCode::PushNull, _) => crate::nan_boxing::TAG_NULL,
                _ => return None,
            }
        } else {
            return None;
        };

        let mut push_count = 0usize;
        for i in (info.header_idx + 1)..info.end_idx {
            let instr = &self.program.instructions[i];
            match (instr.opcode, instr.operand.as_ref()) {
                (OpCode::ArrayPushLocal, Some(Operand::Local(slot))) => {
                    if *slot != push_local || i != push_site_idx {
                        return None;
                    }
                    push_count += 1;
                }
                (OpCode::LoadLocal, Some(Operand::Local(slot)))
                    if *slot == iv_slot || *slot == bound_slot || *slot == push_local => {}
                (OpCode::StoreLocal, Some(Operand::Local(slot)))
                    if *slot == iv_slot || *slot == push_local => {}
                (OpCode::StoreLocal, Some(Operand::Local(_)))
                    if i > info.header_idx + 1
                        && self.program.instructions[i - 1].opcode == OpCode::PushNull => {}
                (OpCode::PushConst, _)
                | (OpCode::PushNull, _)
                | (OpCode::Pop, _)
                | (OpCode::Dup, _)
                | (OpCode::Swap, _)
                | (OpCode::AddInt, _)
                | (OpCode::Add, _)
                | (OpCode::SubInt, _)
                | (OpCode::Sub, _)
                | (OpCode::Gt, _)
                | (OpCode::Lt, _)
                | (OpCode::Gte, _)
                | (OpCode::Lte, _)
                | (OpCode::Eq, _)
                | (OpCode::Neq, _)
                | (OpCode::GtInt, _)
                | (OpCode::LtInt, _)
                | (OpCode::GteInt, _)
                | (OpCode::LteInt, _)
                | (OpCode::EqInt, _)
                | (OpCode::NeqInt, _)
                | (OpCode::Jump, _)
                | (OpCode::JumpIfFalse, _)
                | (OpCode::JumpIfFalseTrusted, _)
                | (OpCode::JumpIfTrue, _) => {}
                _ => return None,
            }
        }

        if push_count == 1 {
            Some(value_bits)
        } else {
            None
        }
    }

    fn emit_loop_entry_array_push_reserve(
        &mut self,
        info: &crate::translator::loop_analysis::LoopInfo,
    ) {
        let (iv_slot, bound_slot, inclusive) =
            if let Some(plan) = self.optimization_plan.loops.get(&info.header_idx)
                && let (Some(iv_slot), Some(bound_slot), Some(1)) =
                    (plan.canonical_iv, plan.bound_slot, plan.step_value)
            {
                let bound_cmp = info
                    .induction_vars
                    .iter()
                    .find(|iv| {
                        !iv.is_module_binding
                            && iv.local_slot == iv_slot
                            && iv.bound_slot == Some(bound_slot)
                    })
                    .map(|iv| iv.bound_cmp)
                    .unwrap_or(IntCC::SignedLessThan);
                let inclusive = matches!(
                    bound_cmp,
                    IntCC::SignedLessThanOrEqual | IntCC::UnsignedLessThanOrEqual
                );
                (iv_slot, bound_slot, inclusive)
            } else if let Some(iv) = info.induction_vars.iter().find(|iv| {
                !iv.is_module_binding && iv.step_value == Some(1) && iv.bound_slot.is_some()
            }) {
                let bound_slot = iv.bound_slot.unwrap();
                let inclusive = matches!(
                    iv.bound_cmp,
                    IntCC::SignedLessThanOrEqual | IntCC::UnsignedLessThanOrEqual
                );
                (iv.local_slot, bound_slot, inclusive)
            } else {
                return;
            };

        let mut push_sites: Vec<(usize, u16)> = Vec::new();
        for i in (info.header_idx + 1)..info.end_idx {
            let instr = &self.program.instructions[i];
            if instr.opcode == OpCode::ArrayPushLocal
                && let Some(Operand::Local(slot)) = instr.operand.as_ref()
            {
                push_sites.push((i, *slot));
            }
        }
        if push_sites.is_empty() {
            return;
        }

        // Keep this path conservative: avoid loops with nested control flow,
        // calls, or direct reassignment to the push target local.
        let has_unsafe_body_shape = ((info.header_idx + 1)..info.end_idx).any(|i| {
            let instr = &self.program.instructions[i];
            matches!(
                instr.opcode,
                OpCode::LoopStart
                    | OpCode::Call
                    | OpCode::CallValue
                    | OpCode::CallMethod
                    | OpCode::DynMethodCall
            )
        });
        if has_unsafe_body_shape {
            return;
        }

        let mut eligible_locals: HashSet<u16> = HashSet::new();
        for &(_, local_slot) in &push_sites {
            let reassigned = ((info.header_idx + 1)..info.end_idx).any(|i| {
                matches!(
                    (&self.program.instructions[i].opcode, self.program.instructions[i].operand.as_ref()),
                    (OpCode::StoreLocal, Some(Operand::Local(slot))) if *slot == local_slot
                )
            });
            if reassigned {
                continue;
            }
            if self.local_initialized_as_array_before_loop(info.header_idx, local_slot) {
                eligible_locals.insert(local_slot);
            }
        }
        if eligible_locals.is_empty() {
            return;
        }

        let bound_i64 = self.read_local_as_i64(bound_slot);
        let required_len = if inclusive {
            self.builder.ins().iadd_imm(bound_i64, 1)
        } else {
            bound_i64
        };
        let zero_i64 = self.builder.ins().iconst(types::I64, 0);
        let positive_required =
            self.builder
                .ins()
                .icmp(IntCC::SignedGreaterThan, required_len, zero_i64);
        let reserve_len = self
            .builder
            .ins()
            .select(positive_required, required_len, zero_i64);

        // Detect push-fill loops and materialize the full array once at loop
        // entry. This removes O(n) push traffic in the loop body while
        // preserving semantics for simple `arr = arr.push(const)` kernels.
        if push_sites.len() == 1
            && self.local_initialized_as_int_before_loop(info.header_idx, iv_slot, 0)
        {
            let (push_site_idx, push_local) = push_sites[0];
            let value_bits = if eligible_locals.contains(&push_local) {
                self.detect_fill_push_value_bits(
                    info,
                    iv_slot,
                    bound_slot,
                    push_site_idx,
                    push_local,
                )
            } else {
                None
            };
            if let Some(value_bits) = value_bits {
                let size_f64 = self.builder.ins().fcvt_from_sint(types::F64, reserve_len);
                let size_bits = self.f64_to_i64(size_f64);
                let value_const = self.builder.ins().iconst(types::I64, value_bits as i64);
                let filled = self
                    .builder
                    .ins()
                    .call(self.ffi.array_filled, &[size_bits, value_const]);
                let filled_arr = self.builder.inst_results(filled)[0];

                let arr_var = self.get_or_create_local(push_local);
                self.builder.def_var(arr_var, filled_arr);

                // Move the IV to loop-final position so the condition fails on
                // the first check and the body is skipped.
                let iv_var = self.get_or_create_local(iv_slot);
                self.builder.def_var(iv_var, size_bits);
            }
        }

        for local_slot in &eligible_locals {
            let var = self.get_or_create_local(*local_slot);
            let arr = self.builder.use_var(var);
            let inst = self
                .builder
                .ins()
                .call(self.ffi.array_reserve_local, &[arr, reserve_len]);
            let updated = self.builder.inst_results(inst)[0];
            self.builder.def_var(var, updated);
        }

        for &(site_idx, local_slot) in &push_sites {
            if eligible_locals.contains(&local_slot) {
                self.trusted_array_push_local_sites.insert(site_idx);
            }
        }

        // Stronger trusted path for fill-style loops:
        // - exactly one push site
        // - IV starts at 0
        // This lets ArrayPushLocal use the IV directly as write index.
        if push_sites.len() == 1
            && self.local_initialized_as_int_before_loop(info.header_idx, iv_slot, 0)
        {
            let (site_idx, local_slot) = push_sites[0];
            if eligible_locals.contains(&local_slot) {
                self.trusted_array_push_local_iv_by_site
                    .insert(site_idx, iv_slot);
            }
        }
    }

    fn emit_linear_bound_guards(&mut self, loop_header: usize) {
        let Some(guards) = self
            .optimization_plan
            .linear_bound_guards_by_loop
            .get(&loop_header)
        else {
            return;
        };
        let guards = guards.clone();
        for LinearBoundGuard {
            array,
            bound_slot,
            inclusive,
        } in guards
        {
            let bound_i64 = self.read_local_as_i64(bound_slot);
            let required_len = if inclusive {
                self.builder.ins().iadd_imm(bound_i64, 1)
            } else {
                bound_i64
            };
            let length = self.read_array_length_for_guard(array);
            let in_bounds =
                self.builder
                    .ins()
                    .icmp(IntCC::UnsignedGreaterThanOrEqual, length, required_len);
            let one = self.builder.ins().iconst(types::I8, 1);
            let zero = self.builder.ins().iconst(types::I8, 0);
            let in_bounds_i8 = self.builder.ins().select(in_bounds, one, zero);
            self.builder.ins().trapz(in_bounds_i8, TrapCode::User(0));
        }
    }

    fn emit_affine_square_bounds_guards(&mut self, loop_header: usize) {
        let Some(guards) = self
            .optimization_plan
            .affine_square_guards_by_loop
            .get(&loop_header)
        else {
            return;
        };

        let guards = guards.clone();
        for guard in guards {
            let bound_i64 = self.read_local_as_i64(guard.bound_slot);
            let required_len = self.builder.ins().imul(bound_i64, bound_i64);
            let length = self.read_array_length_for_guard(guard.array);
            let in_bounds =
                self.builder
                    .ins()
                    .icmp(IntCC::UnsignedGreaterThanOrEqual, length, required_len);
            let one = self.builder.ins().iconst(types::I8, 1);
            let zero = self.builder.ins().iconst(types::I8, 0);
            let in_bounds_i8 = self.builder.ins().select(in_bounds, one, zero);
            self.builder.ins().trapz(in_bounds_i8, TrapCode::User(0));
        }
    }

    pub(crate) fn compile_loop_start(&mut self, idx: usize) -> Result<(), String> {
        if let Some(&end_idx) = self.loop_ends.get(&idx) {
            let start_block = self
                .blocks
                .get(&idx)
                .copied()
                .unwrap_or_else(|| self.builder.create_block());
            let after_end_idx = end_idx + 1;
            let end_block = self
                .blocks
                .get(&after_end_idx)
                .copied()
                .unwrap_or_else(|| self.builder.create_block());

            self.loop_stack.push(LoopContext {
                start_block,
                end_block,
            });
        }

        // GC safepoint poll at loop header.
        //
        // Skip for non-allocating loops: if the loop body contains only pure
        // arithmetic, variable access, and control flow (no heap allocation),
        // GC has nothing to collect and the safepoint poll is wasted overhead.
        // This eliminates a load + compare + branch per iteration (~3 cycles).
        //
        // For loops that CAN allocate, inline the null-pointer check:
        // load gc_safepoint_flag_ptr, if non-null call the slow path.
        // When GC is not configured (ptr == null), this is just a load + branch-
        // not-taken (~2 cycles vs ~15-20 cycles for full FFI call).
        let needs_safepoint = self
            .loop_info
            .get(&idx)
            .map(|info| info.body_can_allocate)
            .unwrap_or(true);

        if needs_safepoint {
            use crate::context::GC_SAFEPOINT_FLAG_PTR_OFFSET;
            let flag_ptr = self.builder.ins().load(
                types::I64,
                MemFlags::trusted(),
                self.ctx_ptr,
                GC_SAFEPOINT_FLAG_PTR_OFFSET,
            );
            let zero = self.builder.ins().iconst(types::I64, 0);
            let needs_gc = self.builder.ins().icmp(IntCC::NotEqual, flag_ptr, zero);
            let gc_block = self.builder.create_block();
            let continue_block = self.builder.create_block();
            self.builder
                .ins()
                .brif(needs_gc, gc_block, &[], continue_block, &[]);
            // GC slow path: call the full safepoint function
            self.builder.switch_to_block(gc_block);
            self.builder.seal_block(gc_block);
            let gc_safepoint = self.ffi.gc_safepoint;
            self.builder.ins().call(gc_safepoint, &[self.ctx_ptr]);
            self.builder.ins().jump(continue_block, &[]);
            // Continue: no GC needed (fast path, branch predicted not-taken)
            self.builder.switch_to_block(continue_block);
            self.builder.seal_block(continue_block);
        }

        // Clear f64 local cache at loop header (block boundary, SSA dominance)
        self.local_f64_cache.clear();

        // LICM: Hoist loop-invariant locals.
        // Pre-load locals that are read but never written inside the loop body.
        // Inside the loop, LoadLocal for these slots reuses the cached value
        // instead of emitting a redundant memory load.
        if let Some(info) = self.loop_info.get(&idx).cloned() {
            // Save outer loop's LICM state before overwriting.
            // This is restored in compile_loop_end so that nested loops
            // don't destroy the outer loop's hoisted optimizations.
            self.hoisted_licm_stack.push(crate::translator::types::SavedLicmState {
                hoisted_locals: std::mem::take(&mut self.hoisted_locals),
                hoisted_array_info: std::mem::take(&mut self.hoisted_array_info),
                hoisted_ref_array_info: std::mem::take(&mut self.hoisted_ref_array_info),
                licm_hoisted_results: std::mem::take(&mut self.licm_hoisted_results),
                licm_skip_indices: std::mem::take(&mut self.licm_skip_indices),
            });
            for &local_idx in &info.invariant_locals {
                let var = self.get_or_create_local(local_idx);
                let val = self.builder.use_var(var);
                self.hoisted_locals.insert(local_idx, val);
            }

            // Array LICM: for call-free loops, pre-extract (data_ptr, length) for
            // invariant array locals used in GetProp or SetIndexRef patterns.
            // This eliminates redundant tag checks + pointer extraction per iteration.
            // (hoisted_array_info and hoisted_ref_array_info are already empty
            //  from std::mem::take above — no need to clear again.)
            {
                let has_calls = ((info.header_idx + 1)..info.end_idx).any(|i| {
                    matches!(
                        self.program.instructions[i].opcode,
                        OpCode::Call
                            | OpCode::CallValue
                            | OpCode::CallMethod
                            | OpCode::DynMethodCall
                    )
                });
                if !has_calls {
                    // Hoist for GetProp array access: LoadLocal(X), <index>, GetProp
                    for &local_idx in &info.invariant_locals {
                        if self.is_used_as_array_in_loop(&info, local_idx) {
                            if std::env::var_os("SHAPE_JIT_UNBOX_LOG").is_some() {
                                eprintln!(
                                    "[shape-jit-array-licm] loop_header={} hoisting direct local={}",
                                    info.header_idx, local_idx
                                );
                            }
                            let var = self.get_or_create_local(local_idx);
                            let arr_boxed = self.builder.use_var(var);
                            let (data_ptr, length) = self.emit_array_data_ptr(arr_boxed);
                            self.hoisted_array_info
                                .insert(local_idx, (data_ptr, length));
                        }
                    }
                    // Hoist for reference-based array access:
                    // - SetIndexRef(ref_slot, ...)
                    // - DerefLoad(ref_slot) ... GetProp
                    let mut ref_candidates: HashSet<u16> =
                        info.invariant_locals.iter().copied().collect();
                    for &local_idx in &info.body_locals_read {
                        if !info.body_locals_written.contains(&local_idx) {
                            ref_candidates.insert(local_idx);
                        }
                    }
                    for i in (info.header_idx + 1)..info.end_idx {
                        let instr = &self.program.instructions[i];
                        if matches!(instr.opcode, OpCode::SetIndexRef | OpCode::DerefLoad)
                            && let Some(shape_vm::bytecode::Operand::Local(local_idx)) =
                                instr.operand.as_ref()
                            && !info.body_locals_written.contains(local_idx)
                        {
                            ref_candidates.insert(*local_idx);
                        }
                    }
                    for local_idx in ref_candidates {
                        let is_set = self.is_ref_used_for_set_index_in_loop(&info, local_idx);
                        let is_get = self.is_ref_used_as_array_in_loop(&info, local_idx);
                        if is_set || is_get {
                            if std::env::var_os("SHAPE_JIT_UNBOX_LOG").is_some() {
                                eprintln!(
                                    "[shape-jit-array-licm] loop_header={} hoisting ref local={} set={} get={}",
                                    info.header_idx, local_idx, is_set, is_get
                                );
                            }
                            let var = self.get_or_create_local(local_idx);
                            let ref_addr = self.builder.use_var(var);
                            let array =
                                self.builder
                                    .ins()
                                    .load(types::I64, MemFlags::new(), ref_addr, 0);
                            let (data_ptr, length) = self.emit_array_data_ptr(array);
                            self.hoisted_ref_array_info
                                .insert(local_idx, (data_ptr, length));
                        }
                    }
                }
            }

            // Call LICM: hoist pure function calls with loop-invariant args.
            // Each hoistable call is emitted once in the pre-header and its result
            // is stored in a Cranelift Variable. The main compilation loop then
            // skips the arg/call instructions and uses the pre-computed result.
            self.emit_licm_hoisted_calls(&info);

            // Emit loop-entry guards for trusted indexed accesses.
            self.emit_loop_entry_array_push_reserve(&info);
            self.emit_non_negative_iv_guards(info.header_idx);
            self.emit_non_negative_step_guards(info.header_idx);
            self.emit_linear_bound_guards(info.header_idx);
            self.emit_affine_square_bounds_guards(info.header_idx);

            // SIMD F64X2 preheader: when the optimizer identified a vectorizable
            // typed-data array loop, emit a vector loop here that processes 2 f64
            // elements per iteration. The scalar loop following handles remainders.
            self.try_emit_simd_preheader(&info);

            let nested_depth = self
                .optimization_plan
                .loops
                .get(&info.header_idx)
                .map(|p| p.nested_depth)
                .unwrap_or(0);
            let vector_width = self
                .optimization_plan
                .vector_width_by_loop
                .get(&info.header_idx)
                .copied()
                .unwrap_or(1);
            let planned_factor_hint_base = self
                .optimization_plan
                .loops
                .get(&info.header_idx)
                .map(|p| p.unroll_factor)
                .unwrap_or(1)
                .max(1)
                .max(vector_width.min(4));
            let affine_numeric_unroll = self.is_affine_numeric_unroll_candidate(&info);
            let planned_factor_hint = if planned_factor_hint_base == 1 && affine_numeric_unroll {
                2
            } else {
                planned_factor_hint_base
            };
            let prefer_nested_unroll =
                affine_numeric_unroll && nested_depth > 0 && planned_factor_hint > 1;
            if std::env::var_os("SHAPE_JIT_UNROLL_LOG").is_some() {
                eprintln!(
                    "[shape-jit-unroll-candidate] loop_header={} nested_depth={} planned_factor_hint={} affine={} prefer_nested_unroll={}",
                    info.header_idx,
                    nested_depth,
                    planned_factor_hint,
                    affine_numeric_unroll,
                    prefer_nested_unroll
                );
            }

            // Integer/float unboxing: for loops with numeric induction variables,
            // convert locals/module bindings from NaN-boxed to raw i64/f64 at loop entry
            // and operate on native types throughout the loop body.
            //
            // Architecture: current block (LoopStart) becomes a prelude that converts
            // NaN-boxed -> raw i64/f64, then jumps to a new inner header block.
            // Back-edges target the inner header (already raw).
            // Both Phi inputs at the inner header carry raw values -> SSA correct.
            //
            // Nested loop support: uses a scope stack to track which locals were
            // newly unboxed at each nesting level. Inner loops inherit outer unboxed
            // state and may add their own. On exit, only the delta is reboxed.
            {
                let (all_int_locals, all_float_locals, all_int_module_bindings) =
                    self.identify_loop_unbox_vars(&info, prefer_nested_unroll);
                if std::env::var_os("SHAPE_JIT_UNBOX_LOG").is_some() {
                    let mut int_locals_vec: Vec<u16> = all_int_locals.iter().copied().collect();
                    let mut float_locals_vec: Vec<u16> = all_float_locals.iter().copied().collect();
                    let mut int_mbs_vec: Vec<u16> =
                        all_int_module_bindings.iter().copied().collect();
                    int_locals_vec.sort_unstable();
                    float_locals_vec.sort_unstable();
                    int_mbs_vec.sort_unstable();
                    eprintln!(
                        "[shape-jit-unbox] loop_header={} int_locals={:?} float_locals={:?} int_module_bindings={:?} already_unboxed_int={:?} already_unboxed_f64={:?}",
                        info.header_idx,
                        int_locals_vec,
                        float_locals_vec,
                        int_mbs_vec,
                        self.unboxed_int_locals.len(),
                        self.unboxed_f64_locals.len()
                    );
                }

                // Compute delta: only locals not already unboxed by an outer scope.
                let int_delta: HashSet<u16> = all_int_locals
                    .difference(&self.unboxed_int_locals)
                    .copied()
                    .collect();
                let float_delta: HashSet<u16> = all_float_locals
                    .difference(&self.unboxed_f64_locals)
                    .copied()
                    .collect();
                let mb_delta: HashSet<u16> = all_int_module_bindings
                    .difference(&self.unboxed_int_module_bindings)
                    .copied()
                    .collect();

                // Detect loop-invariant int locals that feed IntToNumber.
                // These can be pre-converted in the preheader to avoid
                // per-iteration fcvt_from_sint overhead.
                let mut precompute_candidates = Vec::new();
                for &local_idx in &info.invariant_locals {
                    let is_unboxed_int = self.unboxed_int_locals.contains(&local_idx)
                        || int_delta.contains(&local_idx);
                    if !is_unboxed_int {
                        continue;
                    }
                    if self
                        .precomputed_f64_for_invariant_int
                        .contains_key(&local_idx)
                    {
                        continue;
                    }
                    if Self::local_feeds_int_to_number(&self.program.instructions, &info, local_idx)
                    {
                        precompute_candidates.push(local_idx);
                    }
                }

                let has_unbox_delta =
                    !int_delta.is_empty() || !mb_delta.is_empty() || !float_delta.is_empty();
                let has_precompute = !precompute_candidates.is_empty();

                if has_unbox_delta || has_precompute {
                    // Create inner loop header block (receives unboxed values from both paths)
                    let inner_header = self.builder.create_block();

                    // Current block is the prelude (LoopStart block, entry-only path).
                    // Convert only the DELTA locals (newly unboxed at this level).
                    // Locals already unboxed from outer loop are already raw — no conversion needed.
                    for &local_idx in &int_delta {
                        let var = self.get_or_create_local(local_idx);
                        let boxed = self.builder.use_var(var);
                        let f64_val = self.i64_to_f64(boxed);
                        let raw_int = self.builder.ins().fcvt_to_sint_sat(types::I64, f64_val);
                        self.builder.def_var(var, raw_int);
                    }

                    // Float unboxing prelude: create f64-typed Variables for delta only
                    for &local_idx in &float_delta {
                        let i64_var = self.get_or_create_local(local_idx);
                        let boxed = self.builder.use_var(i64_var);
                        let f64_val = self.i64_to_f64(boxed);

                        // Create a new f64-typed Cranelift Variable
                        let f64_var = Variable::new(self.next_var);
                        self.next_var += 1;
                        self.builder.declare_var(f64_var, types::F64);
                        self.builder.def_var(f64_var, f64_val);
                        self.f64_local_vars.insert(local_idx, f64_var);
                    }

                    // Promote module bindings (delta only) to Cranelift Variables:
                    // Load from ctx.locals[] memory, convert to raw i64, def_var.
                    for &mb_idx in &mb_delta {
                        let var = self.get_or_create_module_binding_var(mb_idx);
                        let byte_offset = crate::context::LOCALS_OFFSET + (mb_idx as i32 * 8);
                        let boxed = self.builder.ins().load(
                            types::I64,
                            MemFlags::new(),
                            self.ctx_ptr,
                            byte_offset,
                        );
                        let f64_val = self.i64_to_f64(boxed);
                        let raw_int = self.builder.ins().fcvt_to_sint_sat(types::I64, f64_val);
                        self.builder.def_var(var, raw_int);
                    }

                    // LICM: Pre-convert loop-invariant int locals that feed IntToNumber.
                    {
                        let mut precomputed_this_scope = Vec::new();
                        for local_idx in &precompute_candidates {
                            let var = self.get_or_create_local(*local_idx);
                            let raw_i64 = self.builder.use_var(var);
                            let f64_val = self.builder.ins().fcvt_from_sint(types::F64, raw_i64);
                            let f64_var = Variable::new(self.next_var);
                            self.next_var += 1;
                            self.builder.declare_var(f64_var, types::F64);
                            self.builder.def_var(f64_var, f64_val);
                            self.precomputed_f64_for_invariant_int
                                .insert(*local_idx, f64_var);
                            precomputed_this_scope.push(*local_idx);
                        }
                        self.precomputed_f64_scope_stack
                            .push(precomputed_this_scope);
                    }

                    // Jump from prelude to inner header
                    self.builder.ins().jump(inner_header, &[]);

                    // Seal the prelude block - its only predecessor is the entry edge
                    let prelude_block = self
                        .loop_stack
                        .last()
                        .map(|ctx| ctx.start_block)
                        .unwrap_or_else(|| self.builder.current_block().unwrap());
                    self.builder.seal_block(prelude_block);

                    // Switch to inner header
                    self.builder.switch_to_block(inner_header);

                    // Update back-edge targets so Continue and Jump go to inner header
                    if let Some(loop_ctx) = self.loop_stack.last_mut() {
                        loop_ctx.start_block = inner_header;
                    }
                    self.blocks.insert(idx, inner_header);

                    // Remove the block at idx+1 created by create_blocks_for_jumps.
                    // Without this, the main compile loop would emit a fallthrough from
                    // inner_header to blocks[idx+1], clearing typed_stack and losing
                    // the type info established by the prelude.
                    self.blocks.remove(&(idx + 1));

                    // Update hoisted locals to use raw i64 values from inner header
                    for &local_idx in &int_delta {
                        if self.hoisted_locals.contains_key(&local_idx) {
                            let var = self.get_or_create_local(local_idx);
                            let val = self.builder.use_var(var);
                            self.hoisted_locals.insert(local_idx, val);
                        }
                    }

                    if has_unbox_delta {
                        // Push scope with delta for this nesting level
                        self.unboxed_scope_stack
                            .push(crate::translator::types::UnboxedScope {
                                int_locals: int_delta.clone(),
                                f64_locals: float_delta.clone(),
                                int_module_bindings: mb_delta.clone(),
                                depth: self.loop_stack.len(),
                            });

                        // Union delta into main flat sets
                        self.unboxed_int_locals.extend(&int_delta);
                        self.unboxed_f64_locals.extend(&float_delta);
                        self.unboxed_int_module_bindings.extend(&mb_delta);

                        // Set unboxed_loop_depth only for the outermost scope
                        if self.unboxed_scope_stack.len() == 1 {
                            self.unboxed_loop_depth = self.loop_stack.len();
                        }
                    }
                } else {
                    // No unboxing delta and no precompute candidates — push empty scope.
                    self.precomputed_f64_scope_stack.push(Vec::new());
                }
            }

            // Promote loop-carried module bindings from memory to registers
            // for the loop duration (boxed representation, no type conversion).
            // This is skipped for loops with calls and for currently-unboxed
            // bindings, which are handled by the unboxing path above.
            self.promote_register_carried_module_bindings(&info, idx);

            // Loop unrolling eligibility: single IV(step=1), no nested/calls/alloc, compact body.
            self.pending_unroll = None;
            {
                let canonical_iv =
                    self.optimization_plan
                        .loops
                        .get(&info.header_idx)
                        .and_then(|p| match (p.canonical_iv, p.bound_slot) {
                            (Some(iv), Some(bound)) => Some((iv, bound)),
                            _ => None,
                        });
                if let Some((iv_slot, bound_slot)) = canonical_iv
                    && (!info.body_can_allocate || affine_numeric_unroll)
                {
                    let bound_cmp = info
                        .induction_vars
                        .iter()
                        .find(|iv| {
                            !iv.is_module_binding
                                && iv.local_slot == iv_slot
                                && iv.bound_slot == Some(bound_slot)
                        })
                        .map(|iv| iv.bound_cmp)
                        .unwrap_or(IntCC::SignedLessThan);

                    // Check: no nested loops, no calls, no internal jumps (except condition)
                    let body = &self.program.instructions[(info.header_idx + 1)..info.end_idx];
                    let body_len = body.len();

                    let has_nested = body.iter().any(|i| i.opcode == OpCode::LoopStart);
                    let has_calls = body.iter().any(|i| {
                        matches!(
                            i.opcode,
                            OpCode::Call
                                | OpCode::CallValue
                                | OpCode::CallMethod
                                | OpCode::DynMethodCall
                        )
                    });
                    // ArrayPushLocal is only safe to unroll when this site is
                    // already in the trusted-capacity path. The generic push
                    // path may branch into FFI growth blocks.
                    let has_untrusted_array_push = body.iter().enumerate().any(|(off, i)| {
                        if i.opcode != OpCode::ArrayPushLocal {
                            return false;
                        }
                        let site_idx = info.header_idx + 1 + off;
                        !self.trusted_array_push_local_sites.contains(&site_idx)
                    });
                    // Allow exactly one JumpIfFalse (the condition check) and one Jump (back-edge)
                    let jump_count = body
                        .iter()
                        .filter(|i| {
                            matches!(
                                i.opcode,
                                OpCode::JumpIfFalse
                                    | OpCode::JumpIfFalseTrusted
                                    | OpCode::JumpIfTrue
                                    | OpCode::Break
                                    | OpCode::Continue
                            )
                        })
                        .count();

                    let planned_factor_base = self
                        .optimization_plan
                        .loops
                        .get(&info.header_idx)
                        .map(|p| p.unroll_factor)
                        .unwrap_or(1)
                        .max(1);
                    let vector_width = self
                        .optimization_plan
                        .vector_width_by_loop
                        .get(&info.header_idx)
                        .copied()
                        .unwrap_or(1);
                    let planned_factor_base = planned_factor_base.max(vector_width.min(4));
                    let planned_factor = if planned_factor_base == 1 && affine_numeric_unroll {
                        2
                    } else {
                        planned_factor_base
                    };
                    let body_len_limit = if planned_factor >= 4 {
                        80
                    } else if planned_factor >= 2 {
                        96
                    } else {
                        48
                    };

                    // Integer-unboxed loops can be unrolled only for strict,
                    // affine-safe loop bodies; this avoids SSA instability from
                    // cloning blocks in more complex mixed-state loops.
                    let has_int_unboxed_state = !self.unboxed_int_locals.is_empty()
                        || !self.unboxed_int_module_bindings.is_empty();
                    let int_mbs_empty = self.unboxed_int_module_bindings.is_empty();
                    let body_affine_safe =
                        body.iter().all(|i| is_affine_unroll_safe_opcode(i.opcode));
                    let allow_int_unboxed_unroll = has_int_unboxed_state
                        // Nested affine kernels (e.g. matrix inner loops) are safe
                        // to unroll when the loop body itself has no nested control
                        // flow and only uses affine-safe opcodes.
                        && nested_depth <= 2
                        && int_mbs_empty
                        && planned_factor > 1
                        && body_affine_safe;
                    let allow_nested_affine_unroll = affine_numeric_unroll && nested_depth <= 2;
                    let allow_nested_numeric_unroll = planned_factor > 1 && nested_depth <= 2;
                    let unroll_eligible = (nested_depth == 0
                        || allow_nested_affine_unroll
                        || allow_nested_numeric_unroll)
                        && (!has_int_unboxed_state || allow_int_unboxed_unroll)
                        && planned_factor > 1
                        && !has_nested
                        && !has_calls
                        && !has_untrusted_array_push
                        && jump_count <= 1
                        && body_len <= body_len_limit;
                    if unroll_eligible {
                        // Find JumpIfFalse to determine body_start
                        let mut body_start = None;
                        for i in (info.header_idx + 1)..(info.header_idx + 10).min(info.end_idx) {
                            if matches!(
                                self.program.instructions[i].opcode,
                                OpCode::JumpIfFalse | OpCode::JumpIfFalseTrusted
                            ) {
                                body_start = Some(i + 1);
                                break;
                            }
                        }

                        // Find the back-edge Jump to determine body_end
                        let mut body_end = None;
                        for i in ((info.header_idx + 1)..info.end_idx).rev() {
                            if self.program.instructions[i].opcode == OpCode::Jump {
                                body_end = Some(i);
                                break;
                            }
                        }

                        if let (Some(bs), Some(be)) = (body_start, body_end) {
                            self.pending_unroll = Some(crate::translator::types::UnrollInfo {
                                body_start: bs,
                                body_end: be,
                                iv_slot,
                                bound_slot,
                                bound_cmp,
                                factor: planned_factor,
                            });
                            if std::env::var_os("SHAPE_JIT_UNROLL_LOG").is_some() {
                                eprintln!(
                                    "[shape-jit-unroll] loop_header={} body_len={} iv_slot={} bound_slot={} factor={} nested_depth={} affine={} f64_unboxed={} int_unboxed={}",
                                    info.header_idx,
                                    body_len,
                                    iv_slot,
                                    bound_slot,
                                    planned_factor,
                                    nested_depth,
                                    affine_numeric_unroll,
                                    !self.unboxed_f64_locals.is_empty(),
                                    !self.unboxed_int_locals.is_empty()
                                        || !self.unboxed_int_module_bindings.is_empty()
                                );
                            }
                        }
                    }
                }
            } // use OpCode scope
        }

        Ok(())
    }

    /// Emit a SIMD F64X2 preheader loop for eligible typed-data array loops.
    ///
    /// When the optimizer identified a loop body of the form:
    ///   `dst[i] = src_a[i] <op> src_b[i]` (for i in 0..n)
    /// where all arrays have Float64 typed-data buffers, this method emits
    /// a tight vector loop in the preheader that processes 2 f64 elements
    /// per iteration using 128-bit SIMD (F64X2).
    ///
    /// After the vector loop, the IV is advanced to `n & ~1`, and the normal
    /// scalar loop handles the 0 or 1 remaining elements.
    fn try_emit_simd_preheader(
        &mut self,
        info: &crate::translator::loop_analysis::LoopInfo,
    ) {
        let simd_plan = match self.optimization_plan.simd_plans.get(&info.header_idx) {
            Some(plan) => plan.clone(),
            None => return,
        };

        if std::env::var_os("SHAPE_JIT_SIMD_LOG").is_some() {
            eprintln!(
                "[shape-jit-simd] loop_header={} op={:?} src_a={} src_b={:?} dst={} dst_is_ref={}",
                info.header_idx,
                simd_plan.op,
                simd_plan.src_a_local,
                simd_plan.src_b_local,
                simd_plan.dst_local,
                simd_plan.dst_is_ref,
            );
        }

        // ================================================================
        // Step 1: Extract typed_data pointers for source and destination
        //         arrays in the preheader (before the vector loop).
        // ================================================================

        // Helper: get typed_data pointer from an array local.
        // JitArray layout (repr(C)):
        //   offset  0: data         *mut u64 (boxed buffer)
        //   offset  8: len          u64
        //   offset 24: typed_data   *mut u64 (raw f64 values for Float64 arrays)
        //   offset 32: element_kind u8
        let extract_typed_data = |this: &mut Self, local_slot: u16, is_ref: bool| -> Value {
            let var = this.get_or_create_local(local_slot);
            let boxed = this.builder.use_var(var);

            let arr_boxed = if is_ref {
                // Dereference the reference to get the array value.
                this.builder
                    .ins()
                    .load(types::I64, MemFlags::new(), boxed, 0)
            } else {
                boxed
            };

            // Extract JitArray struct pointer from NaN-boxed heap pointer.
            let arr_ptr = this.emit_jit_alloc_data_ptr(arr_boxed);
            // Load typed_data pointer (offset 24).
            this.builder
                .ins()
                .load(types::I64, MemFlags::trusted(), arr_ptr, 24)
        };

        let src_a_typed = extract_typed_data(self, simd_plan.src_a_local, false);
        let src_b_typed = match simd_plan.src_b_local {
            Some(slot) => Some(extract_typed_data(self, slot, false)),
            None => None,
        };
        let dst_typed = extract_typed_data(self, simd_plan.dst_local, simd_plan.dst_is_ref);

        // Read the loop bound (number of elements).
        let bound_val = self.read_local_as_i64(simd_plan.bound_slot);

        // Compute vec_limit = bound & ~1  (round down to nearest multiple of 2).
        let vec_limit = self.builder.ins().band_imm(bound_val, -2i64);

        // Read IV initial value.
        let iv_initial = self.read_local_as_i64(simd_plan.iv_slot);

        // Check if there's any vectorizable work: iv_initial < vec_limit.
        let has_work = self.builder.ins().icmp(
            IntCC::SignedLessThan,
            iv_initial,
            vec_limit,
        );

        let vec_loop_header = self.builder.create_block();
        let vec_loop_body = self.builder.create_block();
        let vec_loop_exit = self.builder.create_block();

        // Add block params for the IV phi node.
        self.builder
            .append_block_param(vec_loop_header, types::I64);

        // Branch: if has_work, enter vector loop; otherwise skip.
        self.builder.ins().brif(
            has_work,
            vec_loop_header,
            &[iv_initial],
            vec_loop_exit,
            &[],
        );

        // ================================================================
        // Step 2: Vector loop header — phi node for IV.
        // ================================================================
        self.builder.switch_to_block(vec_loop_header);
        let iv_phi = self.builder.block_params(vec_loop_header)[0];

        // Check: iv_phi < vec_limit
        let cond = self
            .builder
            .ins()
            .icmp(IntCC::SignedLessThan, iv_phi, vec_limit);
        self.builder
            .ins()
            .brif(cond, vec_loop_body, &[], vec_loop_exit, &[]);

        // ================================================================
        // Step 3: Vector loop body — load F64X2, operate, store.
        // ================================================================
        self.builder.switch_to_block(vec_loop_body);
        self.builder.seal_block(vec_loop_body);

        // Compute byte offset: iv_phi * 8 (each f64 is 8 bytes).
        let byte_offset = self.builder.ins().ishl_imm(iv_phi, 3);

        // Load 2x f64 from src_a typed_data.
        let addr_a = self.builder.ins().iadd(src_a_typed, byte_offset);
        let vec_a = self
            .builder
            .ins()
            .load(types::F64X2, MemFlags::new(), addr_a, 0);

        // Load or broadcast src_b.
        let vec_b = if let Some(src_b_ptr) = src_b_typed {
            let addr_b = self.builder.ins().iadd(src_b_ptr, byte_offset);
            self.builder
                .ins()
                .load(types::F64X2, MemFlags::new(), addr_b, 0)
        } else {
            // Broadcast scalar — not used in the current analysis but future-proofed.
            let zero = self.builder.ins().f64const(0.0);
            self.builder.ins().splat(types::F64X2, zero)
        };

        // Apply the SIMD operation.
        let vec_result = match simd_plan.op {
            crate::optimizer::vectorization::SIMDOp::Add => {
                self.builder.ins().fadd(vec_a, vec_b)
            }
            crate::optimizer::vectorization::SIMDOp::Sub => {
                self.builder.ins().fsub(vec_a, vec_b)
            }
            crate::optimizer::vectorization::SIMDOp::Mul => {
                self.builder.ins().fmul(vec_a, vec_b)
            }
            crate::optimizer::vectorization::SIMDOp::Div => {
                self.builder.ins().fdiv(vec_a, vec_b)
            }
        };

        // Store result to dst typed_data.
        let addr_dst = self.builder.ins().iadd(dst_typed, byte_offset);
        self.builder
            .ins()
            .store(MemFlags::new(), vec_result, addr_dst, 0);

        // Also update the boxed data buffer for the destination.
        // The dst array's boxed buffer (offset 0 in JitArray) must also be
        // updated so that the boxed view stays in sync with typed_data.
        // For Float64 typed arrays, the boxed buffer stores the same f64 bits
        // as raw u64 (NaN-boxed f64 is a no-op identity).
        {
            let dst_var = self.get_or_create_local(simd_plan.dst_local);
            let dst_boxed = self.builder.use_var(dst_var);
            let dst_arr_boxed = if simd_plan.dst_is_ref {
                self.builder
                    .ins()
                    .load(types::I64, MemFlags::new(), dst_boxed, 0)
            } else {
                dst_boxed
            };
            let dst_arr_ptr = self.emit_jit_alloc_data_ptr(dst_arr_boxed);
            let dst_data_ptr = self
                .builder
                .ins()
                .load(types::I64, MemFlags::trusted(), dst_arr_ptr, 0);
            let boxed_addr = self.builder.ins().iadd(dst_data_ptr, byte_offset);
            // Store the same F64X2 vector to the boxed data buffer.
            // F64 values are stored as u64 in both buffers (no conversion needed
            // since NaN-boxed f64 representation is the identity for normal f64).
            self.builder
                .ins()
                .store(MemFlags::new(), vec_result, boxed_addr, 0);
        }

        // Increment IV by 2.
        let iv_next = self.builder.ins().iadd_imm(iv_phi, 2);
        self.builder
            .ins()
            .jump(vec_loop_header, &[iv_next]);

        // ================================================================
        // Step 4: Vector loop exit — update the scalar IV to vec_limit.
        // ================================================================
        self.builder.switch_to_block(vec_loop_exit);
        self.builder.seal_block(vec_loop_header);
        self.builder.seal_block(vec_loop_exit);

        // Set IV to vec_limit so the scalar remainder loop starts there.
        // We need to write this back to the IV local variable so the
        // subsequent scalar loop picks it up.
        //
        // The IV is either unboxed (raw i64) or NaN-boxed depending on
        // whether integer unboxing has been applied. At this point in
        // compile_loop_start, unboxing hasn't been applied yet, so
        // we write back as NaN-boxed.
        let vec_limit_boxed = {
            let as_f64 = self.builder.ins().fcvt_from_sint(types::F64, vec_limit);
            self.f64_to_i64(as_f64)
        };
        let iv_var = self.get_or_create_local(simd_plan.iv_slot);
        self.builder.def_var(iv_var, vec_limit_boxed);
    }

    pub(crate) fn compile_loop_end(&mut self) -> Result<(), String> {
        // Unboxing: schedule reboxing for the loop's end_block.
        // With scope-stacked unboxing, check if the top scope matches the
        // current loop depth. If so, rebox only the delta (newly unboxed
        // at this level) and restore the outer scope's state.
        if let Some(scope) = self.unboxed_scope_stack.last() {
            if scope.depth == self.loop_stack.len() {
                let scope = self.unboxed_scope_stack.pop().unwrap();

                if !scope.int_locals.is_empty() {
                    self.pending_rebox = Some(scope.int_locals.clone());
                    for local in &scope.int_locals {
                        self.unboxed_int_locals.remove(local);
                    }
                }
                if !scope.int_module_bindings.is_empty() {
                    self.pending_rebox_module_bindings = Some(scope.int_module_bindings.clone());
                    for mb in &scope.int_module_bindings {
                        self.unboxed_int_module_bindings.remove(mb);
                    }
                }
                if !scope.f64_locals.is_empty() {
                    self.pending_rebox_f64 = Some(scope.f64_locals.clone());
                    for local in &scope.f64_locals {
                        self.unboxed_f64_locals.remove(local);
                    }
                }

                // Reset unboxed_loop_depth when all scopes are popped
                if self.unboxed_scope_stack.is_empty() {
                    self.unboxed_loop_depth = 0;
                }
            }
        }
        if self.loop_stack.len() == self.register_carried_loop_depth
            && !self.register_carried_module_bindings.is_empty()
        {
            self.pending_flush_module_bindings =
                Some(self.register_carried_module_bindings.clone());
            self.register_carried_module_bindings.clear();
        }

        self.loop_stack.pop();
        // Restore outer loop's LICM state (saved at LoopStart).
        // This ensures nested loops don't destroy the outer loop's hoisted optimizations.
        if let Some(saved) = self.hoisted_licm_stack.pop() {
            self.hoisted_locals = saved.hoisted_locals;
            self.hoisted_array_info = saved.hoisted_array_info;
            self.hoisted_ref_array_info = saved.hoisted_ref_array_info;
            self.licm_hoisted_results = saved.licm_hoisted_results;
            self.licm_skip_indices = saved.licm_skip_indices;
        } else {
            // No saved state (shouldn't happen, but be safe)
            self.hoisted_locals.clear();
            self.hoisted_array_info.clear();
            self.hoisted_ref_array_info.clear();
            self.licm_hoisted_results.clear();
            self.licm_skip_indices.clear();
        }
        self.local_f64_cache.clear();
        self.pending_unroll = None;
        // Pop precomputed f64 scope: remove entries added at this loop level
        if let Some(precomputed_locals) = self.precomputed_f64_scope_stack.pop() {
            for local_idx in precomputed_locals {
                self.precomputed_f64_for_invariant_int.remove(&local_idx);
            }
        }
        Ok(())
    }

    pub(crate) fn compile_break(&mut self) -> Result<(), String> {
        if let Some(loop_ctx) = self.loop_stack.last() {
            self.builder.ins().jump(loop_ctx.end_block, &[]);
        }
        Ok(())
    }

    pub(crate) fn compile_continue(&mut self) -> Result<(), String> {
        if let Some(loop_ctx) = self.loop_stack.last() {
            self.builder.ins().jump(loop_ctx.start_block, &[]);
        }
        Ok(())
    }

    /// Emit hoisted pure function calls in the loop pre-header.
    ///
    /// For each call identified as hoistable by the LICM analysis:
    /// 1. Read argument values from invariant locals / constants
    /// 2. Emit the call (builtin FFI or inline Cranelift instruction)
    /// 3. Store the result in a new Cranelift Variable
    /// 4. Mark the argument + call instruction indices for skipping
    fn emit_licm_hoisted_calls(
        &mut self,
        info: &crate::translator::loop_analysis::LoopInfo,
    ) {
        use shape_vm::bytecode::{Constant, Operand};

        let hoistable_calls = self
            .optimization_plan
            .licm
            .hoistable_calls_by_loop
            .get(&info.header_idx)
            .cloned()
            .unwrap_or_default();

        if hoistable_calls.is_empty() {
            return;
        }

        for hoist in &hoistable_calls {
            // Collect argument values by reading the arg-push instructions.
            // Each arg instruction is a LoadLocal, LoadLocalTrusted, LoadModuleBinding,
            // PushConst, or PushNull -- all producing a single NaN-boxed i64 Value.
            let mut arg_values: Vec<cranelift::prelude::Value> = Vec::new();
            let mut ok = true;

            for j in hoist.first_arg_idx..(hoist.call_idx - 1) {
                let arg_instr = &self.program.instructions[j];
                let val = match arg_instr.opcode {
                    OpCode::PushConst => {
                        if let Some(Operand::Const(const_idx)) = &arg_instr.operand {
                            match self.program.constants.get(*const_idx as usize) {
                                Some(Constant::Int(v)) => {
                                    let f = *v as f64;
                                    Some(self.f64_const_to_nan_boxed(f))
                                }
                                Some(Constant::UInt(v)) => {
                                    let f = *v as f64;
                                    Some(self.f64_const_to_nan_boxed(f))
                                }
                                Some(Constant::Number(v)) => {
                                    Some(self.f64_const_to_nan_boxed(*v))
                                }
                                Some(Constant::Bool(v)) => {
                                    let tag = if *v {
                                        crate::nan_boxing::TAG_BOOL_TRUE
                                    } else {
                                        crate::nan_boxing::TAG_BOOL_FALSE
                                    };
                                    Some(self.builder.ins().iconst(
                                        cranelift::prelude::types::I64,
                                        tag as i64,
                                    ))
                                }
                                _ => None,
                            }
                        } else {
                            None
                        }
                    }
                    OpCode::PushNull => Some(self.builder.ins().iconst(
                        cranelift::prelude::types::I64,
                        crate::nan_boxing::TAG_NULL as i64,
                    )),
                    OpCode::LoadLocal | OpCode::LoadLocalTrusted => {
                        if let Some(Operand::Local(slot)) = &arg_instr.operand {
                            let var = self.get_or_create_local(*slot);
                            Some(self.builder.use_var(var))
                        } else {
                            None
                        }
                    }
                    OpCode::LoadModuleBinding => {
                        if let Some(Operand::ModuleBinding(slot)) = &arg_instr.operand {
                            let var = self.get_or_create_local(*slot);
                            Some(self.builder.use_var(var))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                match val {
                    Some(v) => arg_values.push(v),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }

            if !ok {
                continue;
            }

            // Emit the call and capture the result.
            // Currently only BuiltinCall is supported for pre-header emission.
            // CallMethod hoisting requires ctx.stack manipulation which is
            // deferred to a future subtask.
            let call_instr = &self.program.instructions[hoist.call_idx];
            let result = match call_instr.opcode {
                OpCode::BuiltinCall => {
                    if let Some(Operand::Builtin(builtin)) = &call_instr.operand {
                        self.emit_licm_builtin_call(builtin, &arg_values)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(result_val) = result {
                // Store result in a new Cranelift Variable.
                let result_var =
                    cranelift::prelude::Variable::new(self.next_var);
                self.next_var += 1;
                self.builder
                    .declare_var(result_var, cranelift::prelude::types::I64);
                self.builder.def_var(result_var, result_val);

                // Register the hoisted result and mark instructions to skip.
                self.licm_hoisted_results
                    .insert(hoist.call_idx, result_var);
                // Skip arg push instructions and the argc PushConst.
                for j in hoist.first_arg_idx..hoist.call_idx {
                    self.licm_skip_indices.insert(j);
                }

                if std::env::var_os("SHAPE_JIT_LICM_LOG").is_some() {
                    eprintln!(
                        "[shape-jit-call-licm] loop_header={} hoisted call at idx={} args={}",
                        info.header_idx,
                        hoist.call_idx,
                        hoist.arg_count,
                    );
                }
            }
        }
    }

    /// Emit a pure builtin call in the loop pre-header for LICM.
    /// Returns the NaN-boxed result Value, or None if the builtin is unsupported.
    fn emit_licm_builtin_call(
        &mut self,
        builtin: &shape_vm::bytecode::BuiltinFunction,
        args: &[cranelift::prelude::Value],
    ) -> Option<cranelift::prelude::Value> {
        use shape_vm::bytecode::BuiltinFunction;

        match builtin {
            // Single-arg Cranelift-native math (no FFI needed)
            BuiltinFunction::Abs if args.len() == 1 => {
                let a_f64 = self.i64_to_f64(args[0]);
                let result_f64 = self.builder.ins().fabs(a_f64);
                Some(self.f64_to_i64(result_f64))
            }
            BuiltinFunction::Sqrt if args.len() == 1 => {
                let a_f64 = self.i64_to_f64(args[0]);
                let result_f64 = self.builder.ins().sqrt(a_f64);
                Some(self.f64_to_i64(result_f64))
            }
            BuiltinFunction::Floor if args.len() == 1 => {
                let a_f64 = self.i64_to_f64(args[0]);
                let result_f64 = self.builder.ins().floor(a_f64);
                Some(self.f64_to_i64(result_f64))
            }
            BuiltinFunction::Ceil if args.len() == 1 => {
                let a_f64 = self.i64_to_f64(args[0]);
                let result_f64 = self.builder.ins().ceil(a_f64);
                Some(self.f64_to_i64(result_f64))
            }
            BuiltinFunction::Round if args.len() == 1 => {
                let a_f64 = self.i64_to_f64(args[0]);
                let result_f64 = self.builder.ins().nearest(a_f64);
                Some(self.f64_to_i64(result_f64))
            }
            // Single-arg trig/transcendental (FFI)
            BuiltinFunction::Sin if args.len() == 1 => {
                let inst = self.builder.ins().call(self.ffi.sin, &[args[0]]);
                Some(self.builder.inst_results(inst)[0])
            }
            BuiltinFunction::Cos if args.len() == 1 => {
                let inst = self.builder.ins().call(self.ffi.cos, &[args[0]]);
                Some(self.builder.inst_results(inst)[0])
            }
            BuiltinFunction::Tan if args.len() == 1 => {
                let inst = self.builder.ins().call(self.ffi.tan, &[args[0]]);
                Some(self.builder.inst_results(inst)[0])
            }
            BuiltinFunction::Asin if args.len() == 1 => {
                let inst = self.builder.ins().call(self.ffi.asin, &[args[0]]);
                Some(self.builder.inst_results(inst)[0])
            }
            BuiltinFunction::Acos if args.len() == 1 => {
                let inst = self.builder.ins().call(self.ffi.acos, &[args[0]]);
                Some(self.builder.inst_results(inst)[0])
            }
            BuiltinFunction::Atan if args.len() == 1 => {
                let inst = self.builder.ins().call(self.ffi.atan, &[args[0]]);
                Some(self.builder.inst_results(inst)[0])
            }
            BuiltinFunction::Exp if args.len() == 1 => {
                let inst = self.builder.ins().call(self.ffi.exp, &[args[0]]);
                Some(self.builder.inst_results(inst)[0])
            }
            BuiltinFunction::Ln if args.len() == 1 => {
                let inst = self.builder.ins().call(self.ffi.ln, &[args[0]]);
                Some(self.builder.inst_results(inst)[0])
            }
            // Two-arg builtins (FFI)
            BuiltinFunction::Log if args.len() == 2 => {
                // log(value, base) -- args[0] is value, args[1] is base
                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.log, &[args[0], args[1]]);
                Some(self.builder.inst_results(inst)[0])
            }
            BuiltinFunction::Pow if args.len() == 2 => {
                // pow(base, exp) -- args[0] is base, args[1] is exp
                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.pow, &[args[0], args[1]]);
                Some(self.builder.inst_results(inst)[0])
            }
            BuiltinFunction::Hypot if args.len() == 2 => {
                // hypot(a, b) = sqrt(a*a + b*b)
                let a_f64 = self.i64_to_f64(args[0]);
                let b_f64 = self.i64_to_f64(args[1]);
                let a2 = self.builder.ins().fmul(a_f64, a_f64);
                let b2 = self.builder.ins().fmul(b_f64, b_f64);
                let sum = self.builder.ins().fadd(a2, b2);
                let result_f64 = self.builder.ins().sqrt(sum);
                Some(self.f64_to_i64(result_f64))
            }
            BuiltinFunction::Sign if args.len() == 1 => {
                // sign via: copysign(1.0, x)
                let a_f64 = self.i64_to_f64(args[0]);
                let one = self.builder.ins().f64const(1.0);
                let result_f64 = self.builder.ins().fcopysign(one, a_f64);
                Some(self.f64_to_i64(result_f64))
            }
            _ => None,
        }
    }

    /// Helper: encode a f64 constant as a NaN-boxed i64 Cranelift Value.
    fn f64_const_to_nan_boxed(&mut self, val: f64) -> cranelift::prelude::Value {
        let bits = val.to_bits() as i64;
        self.builder
            .ins()
            .iconst(cranelift::prelude::types::I64, bits)
    }
}
