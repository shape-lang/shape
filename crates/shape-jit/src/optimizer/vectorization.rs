//! Phase 5: vectorization/strip-mining planning.
//!
//! This module contains two analysis passes:
//! 1. `analyze_vectorization` — strip-mining width analysis (existing Phase 5).
//! 2. `analyze_simd` — F64X2 SIMD lowering for eligible typed-data array loops.

use std::collections::HashMap;

use shape_vm::bytecode::{BytecodeProgram, OpCode, Operand};

use crate::loop_analysis::LoopInfo;

use super::loop_lowering::LoopLoweringPlan;
use super::typed_mir::TypedMirFunction;

/// A vectorizable arithmetic operation on F64 lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SIMDOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Describes an F64X2-vectorizable loop body.
///
/// When present for a loop header, the translator emits a 128-bit SSE2
/// vector body (2x f64 per iteration) with a scalar remainder loop for
/// lengths not divisible by 2.
#[derive(Debug, Clone)]
pub struct SIMDPlan {
    /// The loop header bytecode index.
    pub loop_header: usize,
    /// The single vectorizable operation in the loop body.
    pub op: SIMDOp,
    /// Local slot holding the source array A (invariant, Float64 typed-data).
    pub src_a_local: u16,
    /// Local slot holding the source array B (invariant, Float64 typed-data).
    /// When `None`, src B is a scalar local (broadcast pattern).
    pub src_b_local: Option<u16>,
    /// Local slot holding the destination array (ref-based write target).
    pub dst_local: u16,
    /// Whether the destination is accessed via reference (`SetIndexRef`).
    pub dst_is_ref: bool,
    /// Induction variable local slot.
    pub iv_slot: u16,
    /// Bound local slot (loop iterates `iv < bound`).
    pub bound_slot: u16,
}

fn is_numeric_arith(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::AddDynamic
            | OpCode::SubDynamic
            | OpCode::MulDynamic
            | OpCode::DivDynamic
            | OpCode::ModDynamic
            | OpCode::PowDynamic
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
            | OpCode::NegInt
            | OpCode::NegNumber
    )
}

/// Map a bytecode arithmetic opcode to a SIMDOp, if it represents a simple
/// f64 operation that can be vectorized.
fn opcode_to_simd_op(op: OpCode) -> Option<SIMDOp> {
    match op {
        OpCode::AddDynamic | OpCode::AddNumber => Some(SIMDOp::Add),
        OpCode::SubDynamic | OpCode::SubNumber => Some(SIMDOp::Sub),
        OpCode::MulDynamic | OpCode::MulNumber => Some(SIMDOp::Mul),
        OpCode::DivDynamic | OpCode::DivNumber => Some(SIMDOp::Div),
        _ => None,
    }
}

/// Returns `true` if the opcode is allowed in a SIMD-eligible loop body.
///
/// Only simple control flow, variable access, numeric indexing, and
/// arithmetic are permitted — no calls, allocations, or complex ops.
fn is_simd_body_safe(op: OpCode) -> bool {
    matches!(
        op,
        // Variable access
        OpCode::LoadLocal
            | OpCode::LoadLocalTrusted
            | OpCode::StoreLocal
            | OpCode::StoreLocalTyped
            | OpCode::LoadModuleBinding
            | OpCode::StoreModuleBinding
            // Constants
            | OpCode::PushConst
            | OpCode::PushNull
            // Stack ops
            | OpCode::Pop
            | OpCode::Dup
            | OpCode::Swap
            // Simple f64 arithmetic
            | OpCode::AddDynamic
            | OpCode::SubDynamic
            | OpCode::MulDynamic
            | OpCode::DivDynamic
            | OpCode::AddNumber
            | OpCode::SubNumber
            | OpCode::MulNumber
            | OpCode::DivNumber
            | OpCode::AddInt
            | OpCode::SubInt
            | OpCode::MulInt
            | OpCode::DivInt
            | OpCode::NegInt
            | OpCode::NegNumber
            // Type coercion (numeric)
            | OpCode::IntToNumber
            | OpCode::NumberToInt
            // Comparisons (for loop condition)
            | OpCode::LtDynamic
            | OpCode::LteDynamic
            | OpCode::GtDynamic
            | OpCode::GteDynamic
            | OpCode::LtInt
            | OpCode::LteInt
            | OpCode::GtInt
            | OpCode::GteInt
            | OpCode::LtNumber
            | OpCode::LteNumber
            | OpCode::GtNumber
            | OpCode::GteNumber
            // Control flow (loop structure)
            | OpCode::Jump
            | OpCode::JumpIfFalse
            | OpCode::JumpIfFalseTrusted
            | OpCode::JumpIfTrue
            | OpCode::Break
            | OpCode::Continue
            // Array indexed access
            | OpCode::GetProp
            | OpCode::SetLocalIndex
            | OpCode::SetModuleBindingIndex
            | OpCode::SetIndexRef
            // Reference ops
            | OpCode::MakeRef
            | OpCode::DerefLoad
            | OpCode::DerefStore
            // Length
            | OpCode::Length
            // No-ops in JIT
            | OpCode::Nop
            | OpCode::DropCall
            | OpCode::DropCallAsync
    )
}

/// Analyze loops for SIMD F64X2 lowering eligibility.
///
/// A loop is eligible when:
/// - It has a canonical IV with step=1
/// - It is not nested (depth 0)
/// - The body contains no calls or allocations
/// - All body opcodes are SIMD-safe
/// - The body performs exactly one vectorizable f64 arithmetic op
///   (add/sub/mul/div) on elements loaded from Float64 typed-data arrays
/// - The result is stored to a Float64 typed-data array
/// - Array sources and destination are loop-invariant
pub fn analyze_simd(
    program: &BytecodeProgram,
    loops: &HashMap<usize, LoopInfo>,
    loop_plans: &HashMap<usize, LoopLoweringPlan>,
) -> HashMap<usize, SIMDPlan> {
    let mut out = HashMap::new();

    for (header, info) in loops {
        let Some(loop_plan) = loop_plans.get(header) else {
            continue;
        };

        // Must have canonical IV with step=1 and a known bound slot.
        let (iv_slot, bound_slot) = match (loop_plan.canonical_iv, loop_plan.bound_slot) {
            (Some(iv), Some(bound)) if loop_plan.step_value == Some(1) => (iv, bound),
            _ => continue,
        };

        // No nested loops (keep initial implementation simple).
        if loop_plan.nested_depth > 0 {
            continue;
        }

        // No allocating body.
        if info.body_can_allocate {
            continue;
        }

        // Compact body (avoid vectorizing huge loop bodies).
        let body_len = info.end_idx.saturating_sub(info.header_idx);
        if body_len > 80 {
            continue;
        }

        // All body opcodes must be SIMD-safe (no calls, no complex ops).
        let body_safe = ((info.header_idx + 1)..info.end_idx)
            .all(|i| is_simd_body_safe(program.instructions[i].opcode));
        if !body_safe {
            continue;
        }

        // Scan body for the pattern:
        //   LoadLocal(A), LoadLocal(iv), GetProp   -- load a[i]
        //   LoadLocal(B), LoadLocal(iv), GetProp   -- load b[i]
        //   <arith>                                -- add/sub/mul/div
        //   LoadLocal(iv), SetIndexRef(dst)        -- dst[i] = result
        //   OR: LoadLocal(iv), <value>, SetLocalIndex(dst)
        //
        // We look for exactly 2 GetProp reads + 1 arith + 1 indexed write.

        let mut array_reads: Vec<(u16, usize)> = Vec::new(); // (array_local, instruction_idx)
        let mut arith_ops: Vec<(SIMDOp, usize)> = Vec::new(); // (op, instruction_idx)
        let mut indexed_writes: Vec<(u16, bool, usize)> = Vec::new(); // (dst_local, is_ref, idx)

        for i in (info.header_idx + 1)..info.end_idx {
            let instr = &program.instructions[i];
            match instr.opcode {
                OpCode::GetProp if instr.operand.is_none() => {
                    // Look backward for: LoadLocal(arr_local), LoadLocal(iv), GetProp
                    if i >= 2 {
                        let idx_instr = &program.instructions[i - 1];
                        let arr_instr = &program.instructions[i - 2];

                        let iv_match = matches!(
                            (&idx_instr.opcode, &idx_instr.operand),
                            (OpCode::LoadLocal | OpCode::LoadLocalTrusted, Some(Operand::Local(slot)))
                            if *slot == iv_slot
                        );
                        let arr_local = match (&arr_instr.opcode, &arr_instr.operand) {
                            (
                                OpCode::LoadLocal | OpCode::LoadLocalTrusted,
                                Some(Operand::Local(slot)),
                            ) => Some(*slot),
                            _ => None,
                        };

                        if iv_match {
                            if let Some(arr_slot) = arr_local {
                                if info.invariant_locals.contains(&arr_slot) {
                                    array_reads.push((arr_slot, i));
                                }
                            }
                        }
                    }
                }
                op if opcode_to_simd_op(op).is_some() => {
                    arith_ops.push((opcode_to_simd_op(op).unwrap(), i));
                }
                OpCode::SetIndexRef => {
                    if let Some(Operand::Local(dst_slot)) = &instr.operand {
                        if !info.body_locals_written.contains(dst_slot) {
                            indexed_writes.push((*dst_slot, true, i));
                        }
                    }
                }
                OpCode::SetLocalIndex => {
                    if let Some(Operand::Local(dst_slot)) = &instr.operand {
                        if info.invariant_locals.contains(dst_slot) {
                            indexed_writes.push((*dst_slot, false, i));
                        }
                    }
                }
                _ => {}
            }
        }

        // Require exactly: 2 array reads, 1 arith op, 1 indexed write.
        if array_reads.len() != 2 || arith_ops.len() != 1 || indexed_writes.len() != 1 {
            continue;
        }

        let (src_a, _) = array_reads[0];
        let (src_b, _) = array_reads[1];
        let (simd_op, _) = arith_ops[0];
        let (dst_local, dst_is_ref, _) = indexed_writes[0];

        // Source arrays must be distinct from IV and bound.
        if src_a == iv_slot || src_b == iv_slot || dst_local == iv_slot {
            continue;
        }

        out.insert(
            *header,
            SIMDPlan {
                loop_header: *header,
                op: simd_op,
                src_a_local: src_a,
                src_b_local: Some(src_b),
                dst_local,
                dst_is_ref,
                iv_slot,
                bound_slot,
            },
        );
    }

    out
}

pub fn analyze_vectorization(
    program: &BytecodeProgram,
    loops: &HashMap<usize, LoopInfo>,
    loop_plans: &HashMap<usize, LoopLoweringPlan>,
    typed_mir: &TypedMirFunction,
) -> HashMap<usize, u8> {
    let mut out = HashMap::new();

    for (header, info) in loops {
        let Some(loop_plan) = loop_plans.get(header) else {
            continue;
        };
        if loop_plan.canonical_iv.is_none() {
            continue;
        }
        if info.body_can_allocate {
            continue;
        }
        if loop_plan.nested_depth > 2 {
            continue;
        }

        let mut numeric = 0usize;
        let mut memory = 0usize;
        let mut typed_col_reads = 0usize;
        for i in (info.header_idx + 1)..info.end_idx {
            let op = program.instructions[i].opcode;
            if is_numeric_arith(op) {
                numeric += 1;
            }
            if matches!(
                op,
                OpCode::GetProp
                    | OpCode::SetLocalIndex
                    | OpCode::SetModuleBindingIndex
                    | OpCode::LoadColF64
                    | OpCode::LoadColI64
            ) {
                memory += 1;
            }
            if typed_mir.typed_column_reads.contains(&i) {
                typed_col_reads += 1;
            }
        }

        let body_len = info.end_idx.saturating_sub(info.header_idx);
        let memory_score = memory + typed_col_reads;
        let mut width = 1u8;
        if body_len <= 160 {
            if numeric >= 6 && memory_score >= 1 {
                // Memory + numeric kernels (matrix/spectral style).
                width = if loop_plan.nested_depth <= 1 {
                    4
                } else if loop_plan.nested_depth == 2 && body_len <= 96 {
                    4
                } else {
                    2
                };
            } else if numeric >= 1 && memory_score >= 1 && body_len <= 48 {
                // Memory-dominated scalar kernels (e.g. sieve-style indexed loops).
                // Use light strip-mining even when arithmetic density is low.
                width = 2;
            } else if numeric >= 12 && memory_score == 0 {
                // Pure arithmetic loops (mandelbrot-like) still benefit from
                // strip-mined unroll in absence of true SIMD lane lowering.
                width = if loop_plan.nested_depth <= 1 {
                    4
                } else if loop_plan.nested_depth == 2 && body_len <= 96 {
                    4
                } else {
                    2
                };
            }
        }

        if width > 1 {
            out.insert(*header, width);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cranelift::prelude::IntCC;
    use shape_vm::bytecode::{
        BytecodeProgram, Constant, DebugInfo, Instruction, Operand,
    };

    use crate::loop_analysis::{InductionVar, LoopInfo};

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

    fn make_loop_info(
        header_idx: usize,
        end_idx: usize,
        iv_slot: u16,
        bound_slot: u16,
        invariant_locals: std::collections::HashSet<u16>,
    ) -> LoopInfo {
        LoopInfo {
            header_idx,
            end_idx,
            body_locals_written: {
                let mut s = std::collections::HashSet::new();
                s.insert(iv_slot); // IV is written
                s
            },
            body_locals_read: {
                let mut s = std::collections::HashSet::new();
                s.insert(iv_slot);
                s.insert(bound_slot);
                for &l in &invariant_locals {
                    s.insert(l);
                }
                s
            },
            body_module_bindings_written: std::collections::HashSet::new(),
            body_module_bindings_read: std::collections::HashSet::new(),
            induction_vars: vec![InductionVar {
                local_slot: iv_slot,
                is_module_binding: false,
                bound_cmp: IntCC::SignedLessThan,
                bound_slot: Some(bound_slot),
                step_value: Some(1),
            }],
            invariant_locals,
            invariant_module_bindings: std::collections::HashSet::new(),
            body_can_allocate: false,
            hoistable_calls: vec![],
        }
    }

    fn make_loop_plan(iv_slot: u16, bound_slot: u16) -> LoopLoweringPlan {
        LoopLoweringPlan {
            canonical_iv: Some(iv_slot),
            bound_slot: Some(bound_slot),
            step_value: Some(1),
            nested_depth: 0,
            unroll_factor: 1,
            ..Default::default()
        }
    }

    #[test]
    fn simd_plan_for_elementwise_add() {
        // Loop pattern: for i in 0..n { dst[i] = a[i] + b[i] }
        // Locals: 0=i (IV), 1=n (bound), 2=a, 3=b, 4=dst_ref
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),                        // 0: header
            // Condition: i < n
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),     // 1
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),     // 2
            make_instr(OpCode::LtInt, None),                            // 3
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(12))), // 4
            // Body: dst[i] = a[i] + b[i]
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))),     // 5: load a
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),     // 6: load i
            make_instr(OpCode::GetProp, None),                          // 7: a[i]
            make_instr(OpCode::LoadLocal, Some(Operand::Local(3))),     // 8: load b
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),     // 9: load i
            make_instr(OpCode::GetProp, None),                          // 10: b[i]
            make_instr(OpCode::AddNumber, None),                        // 11: a[i] + b[i]
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),     // 12: load i
            make_instr(OpCode::SetIndexRef, Some(Operand::Local(4))),   // 13: dst[i] = result
            // Increment: i = i + 1
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),     // 14
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),     // 15
            make_instr(OpCode::AddInt, None),                           // 16
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),    // 17
            make_instr(OpCode::LoopEnd, None),                          // 18
        ];

        let program = make_program(instrs, vec![Constant::Int(1)]);
        let mut invariants = std::collections::HashSet::new();
        invariants.insert(1u16); // n
        invariants.insert(2u16); // a
        invariants.insert(3u16); // b
        let info = make_loop_info(0, 18, 0, 1, invariants);
        let plan = make_loop_plan(0, 1);

        let mut loops = HashMap::new();
        loops.insert(0usize, info);
        let mut loop_plans = HashMap::new();
        loop_plans.insert(0usize, plan);

        let simd = analyze_simd(&program, &loops, &loop_plans);
        assert_eq!(simd.len(), 1, "Should find one SIMD-eligible loop");

        let simd_plan = simd.get(&0).unwrap();
        assert_eq!(simd_plan.op, SIMDOp::Add);
        assert_eq!(simd_plan.src_a_local, 2);
        assert_eq!(simd_plan.src_b_local, Some(3));
        assert_eq!(simd_plan.dst_local, 4);
        assert!(simd_plan.dst_is_ref);
        assert_eq!(simd_plan.iv_slot, 0);
        assert_eq!(simd_plan.bound_slot, 1);
    }

    #[test]
    fn simd_plan_rejects_loop_with_call() {
        // Loop with a CallMethod -- should not be SIMD eligible
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
            make_instr(OpCode::LtInt, None),
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(6))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))),
            make_instr(OpCode::CallMethod, Some(Operand::Const(0))),
            make_instr(OpCode::Pop, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),
            make_instr(OpCode::AddInt, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
            make_instr(OpCode::LoopEnd, None),
        ];

        let program = make_program(instrs, vec![Constant::Int(1)]);
        let mut invariants = std::collections::HashSet::new();
        invariants.insert(1u16);
        invariants.insert(2u16);
        let info = make_loop_info(0, 12, 0, 1, invariants);
        let plan = make_loop_plan(0, 1);

        let mut loops = HashMap::new();
        loops.insert(0usize, info);
        let mut loop_plans = HashMap::new();
        loop_plans.insert(0usize, plan);

        let simd = analyze_simd(&program, &loops, &loop_plans);
        assert!(simd.is_empty(), "Loop with CallMethod should not be SIMD eligible");
    }

    #[test]
    fn simd_plan_rejects_step_not_one() {
        // Loop with step=2 -- not eligible (we only handle step=1)
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
            make_instr(OpCode::LtInt, None),
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(4))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),
            make_instr(OpCode::AddInt, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
            make_instr(OpCode::LoopEnd, None),
        ];

        let program = make_program(instrs, vec![Constant::Int(2)]);
        let mut invariants = std::collections::HashSet::new();
        invariants.insert(1u16);
        let info = make_loop_info(0, 9, 0, 1, invariants);
        let mut plan = make_loop_plan(0, 1);
        plan.step_value = Some(2);

        let mut loops = HashMap::new();
        loops.insert(0usize, info);
        let mut loop_plans = HashMap::new();
        loop_plans.insert(0usize, plan);

        let simd = analyze_simd(&program, &loops, &loop_plans);
        assert!(simd.is_empty(), "Loop with step=2 should not be SIMD eligible");
    }

    #[test]
    fn simd_plan_for_elementwise_mul_with_set_local_index() {
        // Loop pattern: for i in 0..n { dst[i] = a[i] * b[i] }
        // Uses SetLocalIndex instead of SetIndexRef
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
            make_instr(OpCode::LtInt, None),
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(12))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::GetProp, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(3))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::GetProp, None),
            make_instr(OpCode::MulNumber, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::SetLocalIndex, Some(Operand::Local(4))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),
            make_instr(OpCode::AddInt, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
            make_instr(OpCode::LoopEnd, None),
        ];

        let program = make_program(instrs, vec![Constant::Int(1)]);
        let mut invariants = std::collections::HashSet::new();
        invariants.insert(1u16);
        invariants.insert(2u16);
        invariants.insert(3u16);
        invariants.insert(4u16); // dst is invariant for SetLocalIndex
        let info = make_loop_info(0, 18, 0, 1, invariants);
        let plan = make_loop_plan(0, 1);

        let mut loops = HashMap::new();
        loops.insert(0usize, info);
        let mut loop_plans = HashMap::new();
        loop_plans.insert(0usize, plan);

        let simd = analyze_simd(&program, &loops, &loop_plans);
        assert_eq!(simd.len(), 1);

        let simd_plan = simd.get(&0).unwrap();
        assert_eq!(simd_plan.op, SIMDOp::Mul);
        assert_eq!(simd_plan.src_a_local, 2);
        assert_eq!(simd_plan.src_b_local, Some(3));
        assert_eq!(simd_plan.dst_local, 4);
        assert!(!simd_plan.dst_is_ref); // SetLocalIndex, not SetIndexRef
    }
}
