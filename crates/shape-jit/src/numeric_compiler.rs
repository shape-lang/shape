//! Legacy numeric-only JIT compiler
//!
//! This module contains the original numeric-only JIT compiler that
//! operates on a pure f64 stack. It's used for simple numeric programs
//! that don't require NaN-boxing or FFI calls.

use cranelift::prelude::*;

use shape_vm::bytecode::{BytecodeProgram, Constant, OpCode, Operand};

/// Compile a simple numeric program to native code
///
/// This is a simpler compiler that only handles numeric operations
/// on an f64 stack, without NaN-boxing or FFI support.
pub fn compile_numeric_program(
    builder: &mut FunctionBuilder,
    program: &BytecodeProgram,
    _stack_ptr: Value,
    _constants_ptr: Value,
) -> Result<Value, String> {
    // Virtual stack for tracking values during compilation
    let mut value_stack: Vec<Value> = Vec::new();

    for instr in &program.instructions {
        match instr.opcode {
            OpCode::PushConst => {
                if let Some(Operand::Const(idx)) = &instr.operand {
                    let const_val = match &program.constants[*idx as usize] {
                        Constant::Number(n) => builder.ins().f64const(*n),
                        Constant::Bool(b) => builder.ins().f64const(if *b { 1.0 } else { 0.0 }),
                        _ => builder.ins().f64const(0.0),
                    };
                    value_stack.push(const_val);
                }
            }
            OpCode::Add => {
                if value_stack.len() >= 2 {
                    let b = value_stack.pop().unwrap();
                    let a = value_stack.pop().unwrap();
                    let result = builder.ins().fadd(a, b);
                    value_stack.push(result);
                }
            }
            OpCode::Sub => {
                if value_stack.len() >= 2 {
                    let b = value_stack.pop().unwrap();
                    let a = value_stack.pop().unwrap();
                    let result = builder.ins().fsub(a, b);
                    value_stack.push(result);
                }
            }
            OpCode::Mul => {
                if value_stack.len() >= 2 {
                    let b = value_stack.pop().unwrap();
                    let a = value_stack.pop().unwrap();
                    let result = builder.ins().fmul(a, b);
                    value_stack.push(result);
                }
            }
            OpCode::Div => {
                if value_stack.len() >= 2 {
                    let b = value_stack.pop().unwrap();
                    let a = value_stack.pop().unwrap();
                    let result = builder.ins().fdiv(a, b);
                    value_stack.push(result);
                }
            }
            OpCode::Neg => {
                if let Some(a) = value_stack.pop() {
                    let result = builder.ins().fneg(a);
                    value_stack.push(result);
                }
            }
            OpCode::Gt => {
                if value_stack.len() >= 2 {
                    let b = value_stack.pop().unwrap();
                    let a = value_stack.pop().unwrap();
                    let cmp = builder.ins().fcmp(FloatCC::GreaterThan, a, b);
                    let true_val = builder.ins().f64const(1.0);
                    let false_val = builder.ins().f64const(0.0);
                    let result = builder.ins().select(cmp, true_val, false_val);
                    value_stack.push(result);
                }
            }
            OpCode::Lt => {
                if value_stack.len() >= 2 {
                    let b = value_stack.pop().unwrap();
                    let a = value_stack.pop().unwrap();
                    let cmp = builder.ins().fcmp(FloatCC::LessThan, a, b);
                    let true_val = builder.ins().f64const(1.0);
                    let false_val = builder.ins().f64const(0.0);
                    let result = builder.ins().select(cmp, true_val, false_val);
                    value_stack.push(result);
                }
            }
            OpCode::Gte => {
                if value_stack.len() >= 2 {
                    let b = value_stack.pop().unwrap();
                    let a = value_stack.pop().unwrap();
                    let cmp = builder.ins().fcmp(FloatCC::GreaterThanOrEqual, a, b);
                    let true_val = builder.ins().f64const(1.0);
                    let false_val = builder.ins().f64const(0.0);
                    let result = builder.ins().select(cmp, true_val, false_val);
                    value_stack.push(result);
                }
            }
            OpCode::Lte => {
                if value_stack.len() >= 2 {
                    let b = value_stack.pop().unwrap();
                    let a = value_stack.pop().unwrap();
                    let cmp = builder.ins().fcmp(FloatCC::LessThanOrEqual, a, b);
                    let true_val = builder.ins().f64const(1.0);
                    let false_val = builder.ins().f64const(0.0);
                    let result = builder.ins().select(cmp, true_val, false_val);
                    value_stack.push(result);
                }
            }
            OpCode::Eq => {
                if value_stack.len() >= 2 {
                    let b = value_stack.pop().unwrap();
                    let a = value_stack.pop().unwrap();
                    let cmp = builder.ins().fcmp(FloatCC::Equal, a, b);
                    let true_val = builder.ins().f64const(1.0);
                    let false_val = builder.ins().f64const(0.0);
                    let result = builder.ins().select(cmp, true_val, false_val);
                    value_stack.push(result);
                }
            }
            OpCode::Neq => {
                if value_stack.len() >= 2 {
                    let b = value_stack.pop().unwrap();
                    let a = value_stack.pop().unwrap();
                    let cmp = builder.ins().fcmp(FloatCC::NotEqual, a, b);
                    let true_val = builder.ins().f64const(1.0);
                    let false_val = builder.ins().f64const(0.0);
                    let result = builder.ins().select(cmp, true_val, false_val);
                    value_stack.push(result);
                }
            }
            OpCode::And => {
                if value_stack.len() >= 2 {
                    let b = value_stack.pop().unwrap();
                    let a = value_stack.pop().unwrap();
                    // Both > 0 means true
                    let zero = builder.ins().f64const(0.0);
                    let a_true = builder.ins().fcmp(FloatCC::GreaterThan, a, zero);
                    let b_true = builder.ins().fcmp(FloatCC::GreaterThan, b, zero);
                    let both = builder.ins().band(a_true, b_true);
                    let true_val = builder.ins().f64const(1.0);
                    let false_val = builder.ins().f64const(0.0);
                    let result = builder.ins().select(both, true_val, false_val);
                    value_stack.push(result);
                }
            }
            OpCode::Or => {
                if value_stack.len() >= 2 {
                    let b = value_stack.pop().unwrap();
                    let a = value_stack.pop().unwrap();
                    let zero = builder.ins().f64const(0.0);
                    let a_true = builder.ins().fcmp(FloatCC::GreaterThan, a, zero);
                    let b_true = builder.ins().fcmp(FloatCC::GreaterThan, b, zero);
                    let either = builder.ins().bor(a_true, b_true);
                    let true_val = builder.ins().f64const(1.0);
                    let false_val = builder.ins().f64const(0.0);
                    let result = builder.ins().select(either, true_val, false_val);
                    value_stack.push(result);
                }
            }
            OpCode::Not => {
                if let Some(a) = value_stack.pop() {
                    let zero = builder.ins().f64const(0.0);
                    let is_zero = builder.ins().fcmp(FloatCC::Equal, a, zero);
                    let true_val = builder.ins().f64const(1.0);
                    let false_val = builder.ins().f64const(0.0);
                    let result = builder.ins().select(is_zero, true_val, false_val);
                    value_stack.push(result);
                }
            }
            OpCode::Dup => {
                if let Some(a) = value_stack.last().cloned() {
                    value_stack.push(a);
                }
            }
            OpCode::Pop => {
                value_stack.pop();
            }
            OpCode::Swap => {
                if value_stack.len() >= 2 {
                    let len = value_stack.len();
                    value_stack.swap(len - 1, len - 2);
                }
            }
            _ => {
                // Unsupported opcode - skip
            }
        }
    }

    // Return top of stack or 0.0
    Ok(value_stack
        .pop()
        .unwrap_or_else(|| builder.ins().f64const(0.0)))
}
