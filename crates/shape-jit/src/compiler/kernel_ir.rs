//! Kernel ABI lowering for the v2 JIT path.
//!
//! This is intentionally not a replacement for the deleted BytecodeToIR
//! translator. The kernel ABI currently has no MIR adapter because `MirToIR`
//! is built around `JITContext`. The only supported bytecode-only shape here is
//! an explicit integer-valued return-code literal, which is enough for the
//! kernel ABI smoke and throughput tests without reintroducing dynamic lowering.

use cranelift::prelude::*;
use shape_vm::bytecode::{BytecodeProgram, Constant, OpCode, Operand};

use crate::context::SimulationKernelConfig;

pub(super) fn build_simulation_kernel_ir(
    builder: &mut FunctionBuilder,
    program: &BytecodeProgram,
    config: &SimulationKernelConfig,
    _cursor_index: Value,
    _series_ptrs: Value,
    _state_ptr: Value,
) -> Result<Value, String> {
    validate_single_series_config(config)?;
    lower_static_return_code(builder, program, "simulation")
}

pub(super) fn build_correlated_kernel_ir(
    builder: &mut FunctionBuilder,
    program: &BytecodeProgram,
    config: &SimulationKernelConfig,
    _cursor_index: Value,
    _series_ptrs: Value,
    _state_ptr: Value,
) -> Result<Value, String> {
    validate_correlated_config(config)?;
    lower_static_return_code(builder, program, "correlated")
}

fn validate_single_series_config(config: &SimulationKernelConfig) -> Result<(), String> {
    if config.is_multi_table() {
        return Err(
            "simulation kernel lowering requires a single-series config; use \
             compile_correlated_kernel for multi-series configs"
                .to_string(),
        );
    }

    for (name, index) in &config.column_map {
        if *index >= config.column_count {
            return Err(format!(
                "simulation kernel config maps column `{}` to index {}, \
                 but column_count is {}",
                name, index, config.column_count
            ));
        }
    }

    Ok(())
}

fn validate_correlated_config(config: &SimulationKernelConfig) -> Result<(), String> {
    if !config.is_multi_table() || config.table_count == 0 {
        return Err(
            "correlated kernel lowering requires a multi-series config with a \
             nonzero table_count"
                .to_string(),
        );
    }

    for (name, index) in &config.table_map {
        if *index >= config.table_count {
            return Err(format!(
                "correlated kernel config maps series `{}` to index {}, \
                 but table_count is {}",
                name, index, config.table_count
            ));
        }
    }

    Ok(())
}

fn lower_static_return_code(
    builder: &mut FunctionBuilder,
    program: &BytecodeProgram,
    mode: &str,
) -> Result<Value, String> {
    let return_code = static_return_code(program, mode)?;
    Ok(builder.ins().iconst(types::I32, i64::from(return_code)))
}

fn static_return_code(program: &BytecodeProgram, mode: &str) -> Result<i32, String> {
    let mut pending_literal = None;

    for (pc, instruction) in program.instructions.iter().enumerate() {
        match (instruction.opcode, instruction.operand) {
            (OpCode::PushConst, Some(Operand::Const(index))) => {
                if pending_literal.is_some() {
                    return Err(format!(
                        "{} kernel lowering only supports one return-code \
                         literal; found another PushConst at instruction {}",
                        mode, pc
                    ));
                }
                let constant = program.constants.get(usize::from(index)).ok_or_else(|| {
                    format!(
                        "{} kernel lowering references missing constant #{} \
                         at instruction {}",
                        mode, index, pc
                    )
                })?;
                pending_literal = Some(return_code_from_constant(constant, mode)?);
            }
            (OpCode::ReturnValue, None) | (OpCode::Halt, None) => {
                return pending_literal.ok_or_else(|| {
                    format!(
                        "{} kernel lowering reached {:?} without a proven \
                         return-code literal",
                        mode, instruction.opcode
                    )
                });
            }
            unsupported => {
                return Err(format!(
                    "{} kernel lowering supports only an explicit constant \
                     return-code program; instruction {} is unsupported: {:?}",
                    mode, pc, unsupported
                ));
            }
        }
    }

    pending_literal.ok_or_else(|| {
        format!(
            "{} kernel lowering requires an explicit integer-valued return-code literal",
            mode
        )
    })
}

fn return_code_from_constant(constant: &Constant, mode: &str) -> Result<i32, String> {
    match constant {
        Constant::Int(value) => i32::try_from(*value).map_err(|_| {
            format!(
                "{} kernel return code {} is outside the i32 ABI range",
                mode, value
            )
        }),
        Constant::UInt(value) => i32::try_from(*value).map_err(|_| {
            format!(
                "{} kernel return code {} is outside the i32 ABI range",
                mode, value
            )
        }),
        Constant::Number(value) => return_code_from_number(*value, mode),
        other => Err(format!(
            "{} kernel return code must be Int, UInt, or integer Number; got {:?}",
            mode, other
        )),
    }
}

fn return_code_from_number(value: f64, mode: &str) -> Result<i32, String> {
    if value.is_finite()
        && value.fract() == 0.0
        && value >= f64::from(i32::MIN)
        && value <= f64::from(i32::MAX)
    {
        Ok(value as i32)
    } else {
        Err(format!(
            "{} kernel return code must be an integer-valued finite number, got {}",
            mode, value
        ))
    }
}
