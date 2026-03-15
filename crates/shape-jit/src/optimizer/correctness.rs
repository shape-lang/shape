//! Phase 8: plan consistency checks.

use shape_vm::bytecode::{BytecodeProgram, OpCode};

use super::AffineGuardArraySource;
use super::FunctionOptimizationPlan;

/// Validate plan invariants early to avoid unsound codegen decisions.
pub fn validate_plan(program: &BytecodeProgram, plan: &FunctionOptimizationPlan) {
    if !cfg!(debug_assertions) {
        return;
    }

    for idx in &plan.trusted_array_get_indices {
        debug_assert!(
            *idx < program.instructions.len(),
            "trusted get index out of range: {idx}"
        );
        debug_assert!(
            program.instructions[*idx].opcode == OpCode::GetProp,
            "trusted get index must point to GetProp: {idx}"
        );
    }

    for idx in &plan.trusted_array_set_indices {
        debug_assert!(
            *idx < program.instructions.len(),
            "trusted set index out of range: {idx}"
        );
        debug_assert!(
            matches!(
                program.instructions[*idx].opcode,
                OpCode::SetLocalIndex | OpCode::SetModuleBindingIndex | OpCode::SetIndexRef
            ),
            "trusted set index must point to indexed set opcode: {idx}"
        );
    }

    for idx in &plan.non_negative_array_get_indices {
        debug_assert!(
            *idx < program.instructions.len(),
            "non-negative get index out of range: {idx}"
        );
        debug_assert!(
            program.instructions[*idx].opcode == OpCode::GetProp,
            "non-negative get index must point to GetProp: {idx}"
        );
    }

    for idx in &plan.non_negative_array_set_indices {
        debug_assert!(
            *idx < program.instructions.len(),
            "non-negative set index out of range: {idx}"
        );
        debug_assert!(
            matches!(
                program.instructions[*idx].opcode,
                OpCode::SetLocalIndex | OpCode::SetModuleBindingIndex | OpCode::SetIndexRef
            ),
            "non-negative set index must point to indexed set opcode: {idx}"
        );
    }

    for header in plan.vector_width_by_loop.keys() {
        debug_assert!(
            plan.loops.contains_key(header),
            "vectorized loop missing loop plan: {header}"
        );
    }

    for (header, guards) in &plan.affine_square_guards_by_loop {
        debug_assert!(
            plan.loops.contains_key(header),
            "affine guard loop missing loop plan: {header}"
        );
        for guard in guards {
            match guard.array {
                AffineGuardArraySource::Local(slot) | AffineGuardArraySource::RefLocal(slot) => {
                    debug_assert!(
                        (slot as usize) < 256,
                        "affine guard local out of range: {slot}"
                    );
                }
                AffineGuardArraySource::ModuleBinding(slot) => {
                    debug_assert!(
                        (slot as usize) < 512,
                        "affine guard module binding out of range: {slot}"
                    );
                }
            }
            debug_assert!(
                (guard.bound_slot as usize) < 64,
                "affine guard bound slot out of range: {}",
                guard.bound_slot
            );
        }
    }

    for (header, guards) in &plan.linear_bound_guards_by_loop {
        debug_assert!(
            plan.loops.contains_key(header),
            "linear guard loop missing loop plan: {header}"
        );
        for guard in guards {
            match guard.array {
                AffineGuardArraySource::Local(slot) | AffineGuardArraySource::RefLocal(slot) => {
                    debug_assert!(
                        (slot as usize) < 256,
                        "linear guard local out of range: {slot}"
                    );
                }
                AffineGuardArraySource::ModuleBinding(slot) => {
                    debug_assert!(
                        (slot as usize) < 512,
                        "linear guard module binding out of range: {slot}"
                    );
                }
            }
            debug_assert!(
                (guard.bound_slot as usize) < 64,
                "linear guard bound slot out of range: {}",
                guard.bound_slot
            );
        }
    }

    for (header, step_slots) in &plan.non_negative_step_guards_by_loop {
        debug_assert!(
            plan.loops.contains_key(header),
            "step guard loop missing loop plan: {header}"
        );
        for slot in step_slots {
            debug_assert!(
                (*slot as usize) < 256,
                "step guard local out of range: {slot}"
            );
        }
    }

    for (header, iv_slots) in &plan.non_negative_iv_guards_by_loop {
        debug_assert!(
            plan.loops.contains_key(header),
            "iv guard loop missing loop plan: {header}"
        );
        for slot in iv_slots {
            debug_assert!(
                (*slot as usize) < 256,
                "iv guard local out of range: {slot}"
            );
        }
    }

    for idx in &plan.numeric_arrays.int_get_sites {
        debug_assert!(
            *idx < program.instructions.len(),
            "numeric int get index out of range: {idx}"
        );
        debug_assert!(
            program.instructions[*idx].opcode == OpCode::GetProp,
            "numeric int get index must point to GetProp: {idx}"
        );
    }

    for idx in &plan.numeric_arrays.float_get_sites {
        debug_assert!(
            *idx < program.instructions.len(),
            "numeric float get index out of range: {idx}"
        );
        debug_assert!(
            program.instructions[*idx].opcode == OpCode::GetProp,
            "numeric float get index must point to GetProp: {idx}"
        );
    }

    for idx in &plan.numeric_arrays.bool_get_sites {
        debug_assert!(
            *idx < program.instructions.len(),
            "bool get index out of range: {idx}"
        );
        debug_assert!(
            program.instructions[*idx].opcode == OpCode::GetProp,
            "bool get index must point to GetProp: {idx}"
        );
    }

    for idx in &plan.numeric_arrays.numeric_set_sites {
        debug_assert!(
            *idx < program.instructions.len(),
            "numeric set index out of range: {idx}"
        );
        debug_assert!(
            matches!(
                program.instructions[*idx].opcode,
                OpCode::SetLocalIndex | OpCode::SetModuleBindingIndex | OpCode::SetIndexRef
            ),
            "numeric set index must point to indexed set opcode: {idx}"
        );
    }

    for idx in &plan.numeric_arrays.bool_set_sites {
        debug_assert!(
            *idx < program.instructions.len(),
            "bool set index out of range: {idx}"
        );
        debug_assert!(
            matches!(
                program.instructions[*idx].opcode,
                OpCode::SetLocalIndex | OpCode::SetModuleBindingIndex | OpCode::SetIndexRef
            ),
            "bool set index must point to indexed set opcode: {idx}"
        );
    }

    for (header, simd_plan) in &plan.simd_plans {
        debug_assert!(
            plan.loops.contains_key(header),
            "SIMD plan loop missing loop plan: {header}"
        );
        debug_assert_eq!(
            simd_plan.loop_header, *header,
            "SIMD plan loop_header mismatch: {} vs {header}",
            simd_plan.loop_header
        );
        let loop_plan = plan.loops.get(header).unwrap();
        debug_assert_eq!(
            loop_plan.canonical_iv,
            Some(simd_plan.iv_slot),
            "SIMD plan IV slot mismatch at loop {header}"
        );
        debug_assert_eq!(
            loop_plan.bound_slot,
            Some(simd_plan.bound_slot),
            "SIMD plan bound slot mismatch at loop {header}"
        );
    }

    for idx in &plan.call_path.prefer_direct_call_sites {
        debug_assert!(
            *idx < program.instructions.len(),
            "direct-call site out of range: {idx}"
        );
        debug_assert!(
            program.instructions[*idx].opcode == OpCode::Call,
            "direct-call site must point to Call: {idx}"
        );
    }

    for (idx, slots) in &plan.call_path.restore_param_slots_by_call_site {
        debug_assert!(
            *idx < program.instructions.len(),
            "restore-slot call site out of range: {idx}"
        );
        debug_assert!(
            program.instructions[*idx].opcode == OpCode::Call,
            "restore-slot site must point to Call: {idx}"
        );
        for slot in slots {
            debug_assert!(
                (*slot as usize) < 256,
                "restore local slot out of range: {slot}"
            );
        }
    }
}
