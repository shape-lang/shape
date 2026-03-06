//! Phase 5: vectorization/strip-mining planning.

use std::collections::HashMap;

use shape_vm::bytecode::{BytecodeProgram, OpCode};

use crate::translator::loop_analysis::LoopInfo;

use super::loop_lowering::LoopLoweringPlan;
use super::typed_mir::TypedMirFunction;

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
