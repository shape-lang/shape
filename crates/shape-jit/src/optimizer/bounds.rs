//! Phase 3: prove array bounds for selected loop accesses.

use std::collections::{HashMap, HashSet};

use cranelift::prelude::IntCC;
use shape_vm::bytecode::{BytecodeProgram, OpCode, Operand};

use crate::loop_analysis::LoopInfo;

use super::loop_lowering::LoopLoweringPlan;

#[derive(Debug, Clone, Default)]
pub struct BoundsPlan {
    pub trusted_get_indices: HashSet<usize>,
    pub trusted_set_indices: HashSet<usize>,
    pub non_negative_get_indices: HashSet<usize>,
    pub non_negative_set_indices: HashSet<usize>,
    pub non_negative_iv_guards_by_loop: HashMap<usize, Vec<u16>>,
    pub non_negative_step_guards_by_loop: HashMap<usize, Vec<u16>>,
    pub linear_bound_guards_by_loop: HashMap<usize, Vec<LinearBoundGuard>>,
    pub affine_square_guards_by_loop: HashMap<usize, Vec<AffineSquareGuard>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArraySource {
    Local(u16),
    ModuleBinding(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AffineGuardArraySource {
    Local(u16),
    RefLocal(u16),
    ModuleBinding(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AffineSquareGuard {
    pub array: AffineGuardArraySource,
    pub bound_slot: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LinearBoundGuard {
    pub array: AffineGuardArraySource,
    pub bound_slot: u16,
    pub inclusive: bool,
}

fn find_length_bound_array_source(
    program: &BytecodeProgram,
    loop_header: usize,
    bound_slot: u16,
) -> Option<ArraySource> {
    if loop_header < 3 {
        return None;
    }

    // Search nearest preheader pattern:
    //   LoadLocal(arr) or LoadModuleBinding(arr)
    //   Length
    //   StoreLocal(bound_slot)
    for i in (2..loop_header).rev() {
        let st = &program.instructions[i];
        let len = &program.instructions[i - 1];
        let ld = &program.instructions[i - 2];

        if st.opcode != OpCode::StoreLocal || len.opcode != OpCode::Length {
            continue;
        }
        let Some(Operand::Local(stored)) = st.operand.as_ref() else {
            continue;
        };
        if *stored != bound_slot {
            continue;
        }
        match (ld.opcode, ld.operand.as_ref()) {
            (OpCode::LoadLocal, Some(Operand::Local(array_local))) => {
                return Some(ArraySource::Local(*array_local));
            }
            (OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(binding))) => {
                return Some(ArraySource::ModuleBinding(*binding));
            }
            _ => {}
        }
    }

    None
}

fn find_ref_slot_for_array_local(
    program: &BytecodeProgram,
    loop_header: usize,
    array_local: u16,
) -> Option<u16> {
    if loop_header < 2 {
        return None;
    }

    // Search nearest preheader pattern:
    //   MakeRef(Local(array_local))
    //   StoreLocal(ref_slot)
    for i in (1..loop_header).rev() {
        let st = &program.instructions[i];
        let mk = &program.instructions[i - 1];
        if st.opcode != OpCode::StoreLocal || mk.opcode != OpCode::MakeRef {
            continue;
        }
        let Some(Operand::Local(ref_slot)) = st.operand.as_ref() else {
            continue;
        };
        let Some(Operand::Local(target_local)) = mk.operand.as_ref() else {
            continue;
        };
        if *target_local == array_local {
            return Some(*ref_slot);
        }
    }
    None
}

fn stack_effect(op: OpCode) -> Option<(i32, i32)> {
    let eff = match op {
        // Push-only
        OpCode::LoadLocal
        | OpCode::LoadLocalTrusted
        | OpCode::LoadModuleBinding
        | OpCode::LoadClosure
        | OpCode::PushConst
        | OpCode::PushNull
        | OpCode::DerefLoad => (0, 1),
        // Unary
        OpCode::IntToNumber
        | OpCode::NumberToInt
        | OpCode::CastWidth
        | OpCode::Neg
        | OpCode::Not
        | OpCode::Length => (1, 1),
        // Binary arithmetic/comparison/indexed read
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
        | OpCode::Gt
        | OpCode::Lt
        | OpCode::Gte
        | OpCode::Lte
        | OpCode::Eq
        | OpCode::Neq
        | OpCode::GtInt
        | OpCode::LtInt
        | OpCode::GteInt
        | OpCode::LteInt
        | OpCode::GtNumber
        | OpCode::LtNumber
        | OpCode::GteNumber
        | OpCode::LteNumber
        | OpCode::EqInt
        | OpCode::EqNumber
        | OpCode::NeqInt
        | OpCode::NeqNumber
        | OpCode::GetProp => (2, 1),
        // Stack shuffles
        OpCode::Dup => (1, 2),
        OpCode::Swap => (2, 2),
        // Pop-only
        OpCode::Pop
        | OpCode::StoreLocal
        | OpCode::StoreLocalTyped
        | OpCode::StoreModuleBinding
        | OpCode::StoreModuleBindingTyped
        | OpCode::StoreClosure
        | OpCode::DerefStore
        | OpCode::DropCall
        | OpCode::DropCallAsync => (1, 0),
        _ => return None,
    };
    Some(eff)
}

fn producer_index_for_stack_pos(
    program: &BytecodeProgram,
    before_idx: usize,
    mut pos_from_top: i32,
) -> Option<usize> {
    for j in (0..before_idx).rev() {
        let (pops, pushes) = stack_effect(program.instructions[j].opcode)?;
        if pos_from_top < pushes {
            return Some(j);
        }
        pos_from_top = pos_from_top - pushes + pops;
        if pos_from_top < 0 {
            return None;
        }
    }
    None
}

fn producer_is_load_local(program: &BytecodeProgram, producer_idx: usize, slot: u16) -> bool {
    let instr = &program.instructions[producer_idx];
    matches!(
        (instr.opcode, instr.operand.as_ref()),
        (OpCode::LoadLocal | OpCode::LoadLocalTrusted, Some(Operand::Local(idx))) if *idx == slot
    )
}

fn producer_is_deref_local(program: &BytecodeProgram, producer_idx: usize, slot: u16) -> bool {
    let instr = &program.instructions[producer_idx];
    matches!(
        (instr.opcode, instr.operand.as_ref()),
        (OpCode::DerefLoad, Some(Operand::Local(idx))) if *idx == slot
    )
}

fn get_prop_array_source(
    program: &BytecodeProgram,
    producer_idx: usize,
) -> Option<AffineGuardArraySource> {
    let instr = &program.instructions[producer_idx];
    match (instr.opcode, instr.operand.as_ref()) {
        (OpCode::LoadLocal | OpCode::LoadLocalTrusted, Some(Operand::Local(slot))) => {
            Some(AffineGuardArraySource::Local(*slot))
        }
        (OpCode::DerefLoad, Some(Operand::Local(slot))) => {
            Some(AffineGuardArraySource::RefLocal(*slot))
        }
        (OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(slot))) => {
            Some(AffineGuardArraySource::ModuleBinding(*slot))
        }
        _ => None,
    }
}

fn is_add_op(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Add | OpCode::AddInt | OpCode::AddNumber
    )
}

fn is_mul_op(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Mul | OpCode::MulInt | OpCode::MulNumber
    )
}

fn producer_local_slot(
    program: &BytecodeProgram,
    producer_idx: usize,
    depth: usize,
) -> Option<u16> {
    if depth > 8 {
        return None;
    }
    let instr = &program.instructions[producer_idx];
    match instr.opcode {
        OpCode::LoadLocal | OpCode::LoadLocalTrusted => match instr.operand.as_ref() {
            Some(Operand::Local(slot)) => Some(*slot),
            _ => None,
        },
        OpCode::IntToNumber | OpCode::NumberToInt | OpCode::CastWidth => {
            let child_idx = producer_index_for_stack_pos(program, producer_idx, 0)?;
            producer_local_slot(program, child_idx, depth + 1)
        }
        _ => None,
    }
}

fn mul_is_iv_times_bound(
    program: &BytecodeProgram,
    producer_idx: usize,
    iv_slots_with_bound: &HashSet<u16>,
    bound_slot: u16,
) -> bool {
    let instr = &program.instructions[producer_idx];
    if !is_mul_op(instr.opcode) {
        return false;
    }
    let Some(lhs_idx) = producer_index_for_stack_pos(program, producer_idx, 1) else {
        return false;
    };
    let Some(rhs_idx) = producer_index_for_stack_pos(program, producer_idx, 0) else {
        return false;
    };
    let lhs_local = producer_local_slot(program, lhs_idx, 0);
    let rhs_local = producer_local_slot(program, rhs_idx, 0);
    matches!(
        (lhs_local, rhs_local),
        (Some(a), Some(b))
            if (a == bound_slot && iv_slots_with_bound.contains(&b))
                || (b == bound_slot && iv_slots_with_bound.contains(&a))
    )
}

fn expr_is_affine_square_index(
    program: &BytecodeProgram,
    producer_idx: usize,
    iv_slots_with_bound: &HashSet<u16>,
    bound_slot: u16,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    let instr = &program.instructions[producer_idx];
    match instr.opcode {
        OpCode::IntToNumber | OpCode::NumberToInt => {
            let Some(child_idx) = producer_index_for_stack_pos(program, producer_idx, 0) else {
                return false;
            };
            expr_is_affine_square_index(
                program,
                child_idx,
                iv_slots_with_bound,
                bound_slot,
                depth + 1,
            )
        }
        _ if is_add_op(instr.opcode) => {
            let Some(lhs_idx) = producer_index_for_stack_pos(program, producer_idx, 1) else {
                return false;
            };
            let Some(rhs_idx) = producer_index_for_stack_pos(program, producer_idx, 0) else {
                return false;
            };
            let lhs_local = producer_local_slot(program, lhs_idx, 0);
            let rhs_local = producer_local_slot(program, rhs_idx, 0);
            (mul_is_iv_times_bound(program, lhs_idx, iv_slots_with_bound, bound_slot)
                && matches!(rhs_local, Some(slot) if iv_slots_with_bound.contains(&slot)))
                || (mul_is_iv_times_bound(program, rhs_idx, iv_slots_with_bound, bound_slot)
                    && matches!(lhs_local, Some(slot) if iv_slots_with_bound.contains(&slot)))
        }
        _ => false,
    }
}

fn constant_is_non_negative(program: &BytecodeProgram, const_idx: u16) -> bool {
    matches!(
        program.constants.get(const_idx as usize),
        Some(shape_vm::bytecode::Constant::Int(v)) if *v >= 0
    ) || matches!(
        program.constants.get(const_idx as usize),
        Some(shape_vm::bytecode::Constant::UInt(_))
    ) || matches!(
        program.constants.get(const_idx as usize),
        Some(shape_vm::bytecode::Constant::Number(v)) if *v >= 0.0
    )
}

fn expr_is_non_negative(
    program: &BytecodeProgram,
    producer_idx: usize,
    non_negative_locals: &HashSet<u16>,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }

    let instr = &program.instructions[producer_idx];
    match instr.opcode {
        OpCode::LoadLocal | OpCode::LoadLocalTrusted => {
            matches!(instr.operand.as_ref(), Some(Operand::Local(slot)) if non_negative_locals.contains(slot))
        }
        OpCode::PushConst => match instr.operand.as_ref() {
            Some(Operand::Const(const_idx)) => constant_is_non_negative(program, *const_idx),
            _ => false,
        },
        OpCode::IntToNumber | OpCode::NumberToInt | OpCode::CastWidth => {
            let Some(operand_idx) = producer_index_for_stack_pos(program, producer_idx, 0) else {
                return false;
            };
            expr_is_non_negative(program, operand_idx, non_negative_locals, depth + 1)
        }
        OpCode::Add
        | OpCode::AddInt
        | OpCode::Mul
        | OpCode::MulInt => {
            let Some(rhs_idx) = producer_index_for_stack_pos(program, producer_idx, 0) else {
                return false;
            };
            let Some(lhs_idx) = producer_index_for_stack_pos(program, producer_idx, 1) else {
                return false;
            };
            if matches!(
                instr.opcode,
                OpCode::Mul | OpCode::MulInt
            ) {
                let lhs_local = producer_local_slot(program, lhs_idx, 0);
                let rhs_local = producer_local_slot(program, rhs_idx, 0);
                if matches!((lhs_local, rhs_local), (Some(l), Some(r)) if l == r) {
                    return true;
                }
            }
            expr_is_non_negative(program, lhs_idx, non_negative_locals, depth + 1)
                && expr_is_non_negative(program, rhs_idx, non_negative_locals, depth + 1)
        }
        _ => false,
    }
}

fn local_init_non_negative_before(
    program: &BytecodeProgram,
    before_idx: usize,
    local_slot: u16,
) -> bool {
    for i in (0..before_idx).rev() {
        let instr = &program.instructions[i];
        if !matches!(
            (instr.opcode, instr.operand.as_ref()),
            (OpCode::StoreLocal, Some(Operand::Local(slot))) if *slot == local_slot
        ) {
            continue;
        }
        let Some(src_idx) = producer_index_for_stack_pos(program, i, 0) else {
            return false;
        };
        return expr_is_non_negative(program, src_idx, &HashSet::new(), 0);
    }
    false
}

fn iv_has_non_negative_progress(
    program: &BytecodeProgram,
    loop_info: &LoopInfo,
    iv_slot: u16,
    step_value: Option<i64>,
) -> Option<Option<u16>> {
    if matches!(step_value, Some(v) if v >= 0) {
        return Some(None);
    }
    if matches!(step_value, Some(v) if v < 0) {
        return None;
    }

    // Handle `iv = iv + step_local` when `step_local` is invariant and
    // either proven non-negative or guarded at loop entry.
    for i in (loop_info.header_idx + 1)..loop_info.end_idx.saturating_sub(3) {
        let load_a = &program.instructions[i];
        let load_b = &program.instructions[i + 1];
        let arith = &program.instructions[i + 2];
        let store = &program.instructions[i + 3];

        if !matches!(
            (store.opcode, store.operand.as_ref()),
            (OpCode::StoreLocal, Some(Operand::Local(slot))) if *slot == iv_slot
        ) {
            continue;
        }
        if !matches!(
            arith.opcode,
            OpCode::Add | OpCode::AddInt
        ) {
            continue;
        }
        let step_slot = match (
            load_a.opcode,
            load_a.operand.as_ref(),
            load_b.opcode,
            load_b.operand.as_ref(),
        ) {
            (
                OpCode::LoadLocal | OpCode::LoadLocalTrusted,
                Some(Operand::Local(a)),
                OpCode::LoadLocal | OpCode::LoadLocalTrusted,
                Some(Operand::Local(b)),
            ) if *a == iv_slot => Some(*b),
            (
                OpCode::LoadLocal | OpCode::LoadLocalTrusted,
                Some(Operand::Local(a)),
                OpCode::LoadLocal | OpCode::LoadLocalTrusted,
                Some(Operand::Local(b)),
            ) if *b == iv_slot => Some(*a),
            _ => None,
        };
        let Some(step_slot) = step_slot else {
            continue;
        };
        if !loop_info.invariant_locals.contains(&step_slot) {
            continue;
        }
        if local_init_non_negative_before(program, loop_info.header_idx, step_slot) {
            return Some(None);
        }
        return Some(Some(step_slot));
    }
    None
}

fn select_iv_for_bounds(
    program: &BytecodeProgram,
    loop_info: &LoopInfo,
) -> Option<(u16, u16, IntCC, Option<u16>)> {
    loop_info.induction_vars.iter().find_map(|iv| {
        if iv.is_module_binding {
            return None;
        }
        let bound_slot = iv.bound_slot?;
        let step_guard =
            iv_has_non_negative_progress(program, loop_info, iv.local_slot, iv.step_value)?;
        Some((iv.local_slot, bound_slot, iv.bound_cmp, step_guard))
    })
}

fn cmp_implies_non_negative_bound(cmp: IntCC) -> bool {
    matches!(cmp, IntCC::SignedLessThan | IntCC::SignedLessThanOrEqual)
}

pub fn analyze_bounds(
    program: &BytecodeProgram,
    loops: &HashMap<usize, LoopInfo>,
    loop_plans: &HashMap<usize, LoopLoweringPlan>,
) -> BoundsPlan {
    let mut plan = BoundsPlan::default();
    let mut non_negative_iv_guards_by_loop: HashMap<usize, HashSet<u16>> = HashMap::new();
    let mut non_negative_step_guards_by_loop: HashMap<usize, HashSet<u16>> = HashMap::new();
    let mut linear_guards_by_loop: HashMap<usize, HashSet<LinearBoundGuard>> = HashMap::new();
    let mut affine_guards_by_loop: HashMap<usize, HashSet<AffineSquareGuard>> = HashMap::new();

    for (header, loop_info) in loops {
        if !loop_plans.contains_key(header) {
            continue;
        }
        let Some((iv_slot, bound_slot, current_cmp, step_guard_slot)) =
            select_iv_for_bounds(program, loop_info)
        else {
            continue;
        };
        if let Some(step_slot) = step_guard_slot {
            non_negative_step_guards_by_loop
                .entry(*header)
                .or_default()
                .insert(step_slot);
        }
        let inclusive_bound = matches!(
            current_cmp,
            IntCC::SignedLessThanOrEqual | IntCC::UnsignedLessThanOrEqual
        );

        let mut non_negative_locals = HashSet::new();
        let mut iv_slots_with_bound = HashSet::new();
        let mut iv_non_negative =
            local_init_non_negative_before(program, loop_info.header_idx, iv_slot)
                && cmp_implies_non_negative_bound(current_cmp);
        let mut bound_non_negative = iv_non_negative;
        if !iv_non_negative && cmp_implies_non_negative_bound(current_cmp) {
            // Runtime guard path: if the IV is >= 0 at loop entry, we can
            // safely skip negative-index normalization in the loop body.
            non_negative_iv_guards_by_loop
                .entry(*header)
                .or_default()
                .insert(iv_slot);
            iv_non_negative = true;
            bound_non_negative = false;
        }
        if iv_non_negative {
            non_negative_locals.insert(iv_slot);
            if bound_non_negative {
                non_negative_locals.insert(bound_slot);
                iv_slots_with_bound.insert(iv_slot);
            }
        }
        for (outer_header, outer_loop_info) in loops {
            if outer_loop_info.header_idx >= loop_info.header_idx
                || outer_loop_info.end_idx <= loop_info.end_idx
            {
                continue;
            }
            let Some(_outer_plan) = loop_plans.get(outer_header) else {
                continue;
            };
            let Some((outer_iv, outer_bound, outer_cmp, _outer_step_guard_slot)) =
                select_iv_for_bounds(program, outer_loop_info)
            else {
                continue;
            };
            if !cmp_implies_non_negative_bound(outer_cmp)
                || !local_init_non_negative_before(program, outer_loop_info.header_idx, outer_iv)
            {
                continue;
            }
            non_negative_locals.insert(outer_iv);
            non_negative_locals.insert(outer_bound);
            if outer_bound == bound_slot {
                iv_slots_with_bound.insert(outer_iv);
            }
        }

        let array_source =
            find_length_bound_array_source(program, loop_info.header_idx, bound_slot);
        let maybe_ref_slot = match array_source {
            Some(ArraySource::Local(local)) => {
                find_ref_slot_for_array_local(program, loop_info.header_idx, local)
            }
            _ => None,
        };

        // Proven reads:
        //   Load{Local|ModuleBinding}(array), LoadLocal(iv_slot), GetProp
        for i in (loop_info.header_idx + 1)..loop_info.end_idx {
            let instr = &program.instructions[i];
            match instr.opcode {
                OpCode::GetProp if instr.operand.is_none() => {
                    let Some(obj_src_idx) = producer_index_for_stack_pos(program, i, 1) else {
                        continue;
                    };
                    let Some(key_src_idx) = producer_index_for_stack_pos(program, i, 0) else {
                        continue;
                    };

                    let arr_ok = match array_source {
                        Some(ArraySource::Local(local)) => {
                            producer_is_load_local(program, obj_src_idx, local)
                                || matches!(
                                    maybe_ref_slot,
                                    Some(ref_slot)
                                        if producer_is_deref_local(program, obj_src_idx, ref_slot)
                                )
                        }
                        Some(ArraySource::ModuleBinding(binding)) => matches!(
                            (
                                program.instructions[obj_src_idx].opcode,
                                program.instructions[obj_src_idx].operand.as_ref()
                            ),
                            (OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(slot)))
                                if *slot == binding
                        ),
                        None => false,
                    };
                    let idx_ok = producer_is_load_local(program, key_src_idx, iv_slot);
                    if arr_ok && idx_ok && iv_non_negative {
                        plan.trusted_get_indices.insert(i);
                    }

                    if idx_ok && expr_is_non_negative(program, key_src_idx, &non_negative_locals, 0)
                    {
                        if let Some(array) = get_prop_array_source(program, obj_src_idx) {
                            plan.trusted_get_indices.insert(i);
                            linear_guards_by_loop.entry(*header).or_default().insert(
                                LinearBoundGuard {
                                    array,
                                    bound_slot,
                                    inclusive: inclusive_bound,
                                },
                            );
                        }
                    }

                    if expr_is_affine_square_index(
                        program,
                        key_src_idx,
                        &iv_slots_with_bound,
                        bound_slot,
                        0,
                    ) && expr_is_non_negative(program, key_src_idx, &non_negative_locals, 0)
                    {
                        if let Some(array) = get_prop_array_source(program, obj_src_idx) {
                            plan.trusted_get_indices.insert(i);
                            affine_guards_by_loop
                                .entry(*header)
                                .or_default()
                                .insert(AffineSquareGuard { array, bound_slot });
                        }
                    }

                    let obj_is_ref_based =
                        matches!(program.instructions[obj_src_idx].opcode, OpCode::DerefLoad);
                    if expr_is_non_negative(program, key_src_idx, &non_negative_locals, 0)
                        && (arr_ok || obj_is_ref_based)
                    {
                        plan.non_negative_get_indices.insert(i);
                    }
                }
                OpCode::SetLocalIndex => {
                    let Some(key_src_idx) = producer_index_for_stack_pos(program, i, 1) else {
                        continue;
                    };
                    let arr_ok = matches!(
                        (array_source, instr.operand.as_ref()),
                        (Some(ArraySource::Local(array_local)), Some(Operand::Local(slot)))
                            if *slot == array_local
                    );
                    let idx_ok = producer_is_load_local(program, key_src_idx, iv_slot);
                    if arr_ok && idx_ok && iv_non_negative {
                        plan.trusted_set_indices.insert(i);
                    }
                    if idx_ok && expr_is_non_negative(program, key_src_idx, &non_negative_locals, 0)
                    {
                        if let Some(Operand::Local(slot)) = instr.operand.as_ref() {
                            plan.trusted_set_indices.insert(i);
                            linear_guards_by_loop.entry(*header).or_default().insert(
                                LinearBoundGuard {
                                    array: AffineGuardArraySource::Local(*slot),
                                    bound_slot,
                                    inclusive: inclusive_bound,
                                },
                            );
                        }
                    }
                    if expr_is_non_negative(program, key_src_idx, &non_negative_locals, 0) {
                        plan.non_negative_set_indices.insert(i);
                    }
                }
                OpCode::SetModuleBindingIndex => {
                    let Some(key_src_idx) = producer_index_for_stack_pos(program, i, 1) else {
                        continue;
                    };
                    let arr_ok = matches!(
                        (array_source, instr.operand.as_ref()),
                        (
                            Some(ArraySource::ModuleBinding(array_binding)),
                            Some(Operand::ModuleBinding(slot))
                        ) if *slot == array_binding
                    );
                    let idx_ok = producer_is_load_local(program, key_src_idx, iv_slot);
                    if arr_ok && idx_ok && iv_non_negative {
                        plan.trusted_set_indices.insert(i);
                    }
                    if idx_ok && expr_is_non_negative(program, key_src_idx, &non_negative_locals, 0)
                    {
                        if let Some(Operand::ModuleBinding(slot)) = instr.operand.as_ref() {
                            plan.trusted_set_indices.insert(i);
                            linear_guards_by_loop.entry(*header).or_default().insert(
                                LinearBoundGuard {
                                    array: AffineGuardArraySource::ModuleBinding(*slot),
                                    bound_slot,
                                    inclusive: inclusive_bound,
                                },
                            );
                        }
                    }
                    if expr_is_non_negative(program, key_src_idx, &non_negative_locals, 0) {
                        plan.non_negative_set_indices.insert(i);
                    }
                }
                OpCode::SetIndexRef => {
                    let Some(key_src_idx) = producer_index_for_stack_pos(program, i, 1) else {
                        continue;
                    };
                    let ref_ok = matches!(
                        (maybe_ref_slot, instr.operand.as_ref()),
                        (Some(ref_slot), Some(Operand::Local(slot))) if *slot == ref_slot
                    );
                    let idx_ok = producer_is_load_local(program, key_src_idx, iv_slot);
                    if ref_ok && idx_ok && iv_non_negative {
                        plan.trusted_set_indices.insert(i);
                    }
                    if idx_ok && expr_is_non_negative(program, key_src_idx, &non_negative_locals, 0)
                    {
                        if let Some(Operand::Local(slot)) = instr.operand.as_ref() {
                            plan.trusted_set_indices.insert(i);
                            linear_guards_by_loop.entry(*header).or_default().insert(
                                LinearBoundGuard {
                                    array: AffineGuardArraySource::RefLocal(*slot),
                                    bound_slot,
                                    inclusive: inclusive_bound,
                                },
                            );
                        }
                    }
                    if expr_is_non_negative(program, key_src_idx, &non_negative_locals, 0) {
                        plan.non_negative_set_indices.insert(i);
                    }
                }
                _ => {}
            }
        }
    }

    for (header, guards) in linear_guards_by_loop {
        let mut guards_vec: Vec<_> = guards.into_iter().collect();
        guards_vec.sort_by_key(|guard| {
            let (kind, slot) = match guard.array {
                AffineGuardArraySource::Local(slot) => (0u8, slot),
                AffineGuardArraySource::RefLocal(slot) => (1u8, slot),
                AffineGuardArraySource::ModuleBinding(slot) => (2u8, slot),
            };
            (kind, slot, guard.bound_slot, guard.inclusive as u8)
        });
        plan.linear_bound_guards_by_loop.insert(header, guards_vec);
    }

    for (header, guards) in non_negative_step_guards_by_loop {
        let mut guards_vec: Vec<_> = guards.into_iter().collect();
        guards_vec.sort_unstable();
        plan.non_negative_step_guards_by_loop
            .insert(header, guards_vec);
    }

    for (header, guards) in non_negative_iv_guards_by_loop {
        let mut guards_vec: Vec<_> = guards.into_iter().collect();
        guards_vec.sort_unstable();
        plan.non_negative_iv_guards_by_loop
            .insert(header, guards_vec);
    }

    for (header, guards) in affine_guards_by_loop {
        let mut guards_vec: Vec<_> = guards.into_iter().collect();
        guards_vec.sort_by_key(|guard| {
            let (kind, slot) = match guard.array {
                AffineGuardArraySource::Local(slot) => (0u8, slot),
                AffineGuardArraySource::RefLocal(slot) => (1u8, slot),
                AffineGuardArraySource::ModuleBinding(slot) => (2u8, slot),
            };
            (kind, slot, guard.bound_slot)
        });
        plan.affine_square_guards_by_loop.insert(header, guards_vec);
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loop_analysis;
    use shape_vm::bytecode::{BytecodeProgram, Constant, DebugInfo, Instruction};

    fn make_instr(opcode: OpCode, operand: Option<Operand>) -> Instruction {
        Instruction { opcode, operand }
    }

    fn make_program(instrs: Vec<Instruction>, constants: Vec<Constant>) -> BytecodeProgram {
        BytecodeProgram {
            instructions: instrs,
            constants,
            strings: vec![],
            functions: vec![],
            debug_info: DebugInfo::default(),
            data_schema: None,
            module_binding_names: vec![],
            top_level_locals_count: 0,
            top_level_local_storage_hints: vec![],
            type_schema_registry: Default::default(),
            module_binding_storage_hints: vec![],
            function_local_storage_hints: vec![],
            compiled_annotations: Default::default(),
            trait_method_symbols: Default::default(),
            expanded_function_defs: Default::default(),
            string_index: Default::default(),
            foreign_functions: vec![],
            native_struct_layouts: vec![],
            content_addressed: None,
            function_blob_hashes: vec![],
            top_level_frame: None,
            ..Default::default()
        }
    }

    #[test]
    fn trusts_module_binding_index_writes_when_iv_and_bound_match() {
        let instrs = vec![
            make_instr(OpCode::PushConst, Some(Operand::Const(0))), // i = 0
            make_instr(OpCode::StoreLocal, Some(Operand::Local(1))),
            make_instr(OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(0))),
            make_instr(OpCode::Length, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(2))), // n = len(arr)
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // i
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))), // n
            make_instr(OpCode::LtInt, None),
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(8))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // i
            make_instr(OpCode::PushConst, Some(Operand::Const(0))), // value
            make_instr(
                OpCode::SetModuleBindingIndex,
                Some(Operand::ModuleBinding(0)),
            ),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
            make_instr(OpCode::PushConst, Some(Operand::Const(1))),
            make_instr(OpCode::AddInt, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(1))),
            make_instr(OpCode::Jump, Some(Operand::Offset(-12))),
            make_instr(OpCode::LoopEnd, None),
        ];
        let program = make_program(instrs, vec![Constant::Int(0), Constant::Int(1)]);
        let loops = loop_analysis::analyze_loops(&program);
        let loop_plans = super::super::loop_lowering::plan_loops(
            &program,
            &loops,
            &super::super::typed_mir::build_typed_mir(&program),
        );
        let bounds = analyze_bounds(&program, &loops, &loop_plans);

        assert!(
            bounds.trusted_set_indices.contains(&12),
            "expected SetModuleBindingIndex at 12 to be trusted"
        );
    }

    #[test]
    fn trusts_set_index_ref_when_ref_targets_length_bounded_local_array() {
        let instrs = vec![
            make_instr(OpCode::PushConst, Some(Operand::Const(0))), // i = 0
            make_instr(OpCode::StoreLocal, Some(Operand::Local(1))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))), // arr
            make_instr(OpCode::Length, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(2))), // n = len(arr)
            make_instr(OpCode::MakeRef, Some(Operand::Local(0))),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(3))), // r = &arr
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // i
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))), // n
            make_instr(OpCode::LtInt, None),
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(8))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // i
            make_instr(OpCode::PushConst, Some(Operand::Const(0))), // value
            make_instr(OpCode::SetIndexRef, Some(Operand::Local(3))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
            make_instr(OpCode::PushConst, Some(Operand::Const(1))),
            make_instr(OpCode::AddInt, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(1))),
            make_instr(OpCode::Jump, Some(Operand::Offset(-12))),
            make_instr(OpCode::LoopEnd, None),
        ];
        let program = make_program(instrs, vec![Constant::Int(0), Constant::Int(1)]);
        let loops = loop_analysis::analyze_loops(&program);
        let loop_plans = super::super::loop_lowering::plan_loops(
            &program,
            &loops,
            &super::super::typed_mir::build_typed_mir(&program),
        );
        let bounds = analyze_bounds(&program, &loops, &loop_plans);

        assert!(
            bounds.trusted_set_indices.contains(&14),
            "expected SetIndexRef at 14 to be trusted"
        );
    }

    #[test]
    fn marks_non_negative_get_for_affine_index_expr() {
        // i starts at 0 and increments by 1 under i < n.
        // Index expression i * n + i is non-negative in-loop.
        let instrs = vec![
            make_instr(OpCode::PushConst, Some(Operand::Const(0))), // i = 0
            make_instr(OpCode::StoreLocal, Some(Operand::Local(1))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))), // arr
            make_instr(OpCode::Length, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(2))), // n = len(arr)
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // i
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))), // n
            make_instr(OpCode::LtInt, None),
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(11))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))), // arr
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // i
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))), // n
            make_instr(OpCode::MulInt, None),                       // i * n
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // i
            make_instr(OpCode::AddInt, None),                       // i * n + i
            make_instr(OpCode::GetProp, None),
            make_instr(OpCode::Pop, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
            make_instr(OpCode::PushConst, Some(Operand::Const(1))),
            make_instr(OpCode::AddInt, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(1))),
            make_instr(OpCode::Jump, Some(Operand::Offset(-17))),
            make_instr(OpCode::LoopEnd, None),
        ];
        let program = make_program(instrs, vec![Constant::Int(0), Constant::Int(1)]);
        let loops = loop_analysis::analyze_loops(&program);
        let loop_plans = super::super::loop_lowering::plan_loops(
            &program,
            &loops,
            &super::super::typed_mir::build_typed_mir(&program),
        );
        let bounds = analyze_bounds(&program, &loops, &loop_plans);

        assert!(
            bounds.non_negative_get_indices.contains(&16),
            "expected GetProp at 16 to be marked non-negative"
        );
    }

    #[test]
    fn adds_iv_non_negative_guard_and_trusts_ref_set_with_linear_bound() {
        // iv starts from an unknown local, so static non-negative proof fails.
        // The planner should add a loop-entry iv>=0 guard, then allow trusted
        // SetIndexRef with a linear length guard against bound_slot.
        let instrs = vec![
            make_instr(OpCode::LoadLocal, Some(Operand::Local(5))), // unknown iv seed
            make_instr(OpCode::StoreLocal, Some(Operand::Local(1))), // iv
            make_instr(OpCode::LoadLocal, Some(Operand::Local(6))), // unknown bound seed
            make_instr(OpCode::StoreLocal, Some(Operand::Local(2))), // bound
            make_instr(OpCode::MakeRef, Some(Operand::Local(0))),   // ref source
            make_instr(OpCode::StoreLocal, Some(Operand::Local(3))), // ref slot
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // iv
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))), // bound
            make_instr(OpCode::LteInt, None),
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(8))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // key = iv
            make_instr(OpCode::PushConst, Some(Operand::Const(0))), // value
            make_instr(OpCode::SetIndexRef, Some(Operand::Local(3))), // idx 13
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
            make_instr(OpCode::PushConst, Some(Operand::Const(1))),
            make_instr(OpCode::AddInt, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(1))),
            make_instr(OpCode::Jump, Some(Operand::Offset(-12))),
            make_instr(OpCode::LoopEnd, None),
        ];
        let program = make_program(instrs, vec![Constant::Int(0), Constant::Int(1)]);
        let loops = loop_analysis::analyze_loops(&program);
        let loop_plans = super::super::loop_lowering::plan_loops(
            &program,
            &loops,
            &super::super::typed_mir::build_typed_mir(&program),
        );
        let bounds = analyze_bounds(&program, &loops, &loop_plans);

        let iv_guards = bounds
            .non_negative_iv_guards_by_loop
            .get(&6)
            .expect("expected iv guard for loop header 6");
        assert_eq!(iv_guards, &vec![1]);
        assert!(
            bounds.trusted_set_indices.contains(&13),
            "expected SetIndexRef at 13 to be trusted with iv guard + linear bound guard"
        );
        let linear_guards = bounds
            .linear_bound_guards_by_loop
            .get(&6)
            .expect("expected linear bound guard for loop header 6");
        assert!(linear_guards.contains(&LinearBoundGuard {
            array: AffineGuardArraySource::RefLocal(3),
            bound_slot: 2,
            inclusive: true,
        }));
    }
}
