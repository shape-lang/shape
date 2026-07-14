//! Numeric opcode metrics for whole-program compilation.

use std::collections::BTreeMap;

use shape_vm::bytecode::{BytecodeProgram, OpCode};

#[derive(Default)]
struct NumericOpcodeStats {
    typed: usize,
    generic: usize,
    typed_breakdown: BTreeMap<String, usize>,
    generic_breakdown: BTreeMap<String, usize>,
}

fn bump_breakdown(map: &mut BTreeMap<String, usize>, opcode: OpCode) {
    *map.entry(format!("{opcode:?}")).or_insert(0) += 1;
}

fn collect_numeric_opcode_stats(program: &BytecodeProgram) -> NumericOpcodeStats {
    let mut stats = NumericOpcodeStats::default();
    for instruction in &program.instructions {
        match instruction.opcode {
            OpCode::AddInt
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
            | OpCode::EqString
            | OpCode::GtString
            | OpCode::LtString
            | OpCode::GteString
            | OpCode::LteString
            | OpCode::EqDecimal
            | OpCode::IsNull
            | OpCode::NegInt
            | OpCode::NegNumber => {
                stats.typed += 1;
                bump_breakdown(&mut stats.typed_breakdown, instruction.opcode);
            }
            _ => {}
        }
    }
    stats
}

pub(super) fn maybe_emit_numeric_metrics(program: &BytecodeProgram) {
    if !tracing::enabled!(target: "shape_jit::metrics", tracing::Level::INFO) {
        return;
    }
    let stats = collect_numeric_opcode_stats(program);
    let total = stats.typed + stats.generic;
    let coverage = if total == 0 {
        100.0
    } else {
        (stats.typed as f64 * 100.0) / (total as f64)
    };
    tracing::info!(
        target: "shape_jit::metrics",
        typed_numeric_ops = stats.typed,
        generic_numeric_ops = stats.generic,
        typed_numeric_coverage_pct = coverage,
        static_typed_numeric_ops = stats.typed,
        static_generic_numeric_ops = stats.generic,
        static_typed_numeric_coverage_pct = coverage,
        "shape-jit-metrics numeric coverage",
    );
    if tracing::enabled!(target: "shape_jit::metrics", tracing::Level::TRACE) {
        let format_breakdown = |breakdown: &BTreeMap<String, usize>| {
            breakdown
                .iter()
                .map(|(name, count)| format!("{name}:{count}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        tracing::trace!(
            target: "shape_jit::metrics",
            typed_breakdown = %format_breakdown(&stats.typed_breakdown),
            generic_breakdown = %format_breakdown(&stats.generic_breakdown),
            "shape-jit-metrics-detail breakdown",
        );
    }
}
