//! Literal and operator compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use shape_ast::ast::{BinaryOp, Literal, UnaryOp};
use shape_ast::error::{Result, ShapeError};

use super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a literal value
    pub(super) fn compile_literal(&mut self, lit: &Literal) -> Result<()> {
        if let Literal::FormattedString { value, mode } = lit {
            return self.compile_interpolated_string_expression(value, *mode);
        }

        let const_val = match lit {
            Literal::Int(i) => Some(Constant::Int(*i)),
            Literal::UInt(u) => {
                if *u <= i64::MAX as u64 {
                    Some(Constant::Int(*u as i64))
                } else {
                    Some(Constant::UInt(*u))
                }
            }
            Literal::TypedInt(v, _) => Some(Constant::Int(*v)),
            Literal::Number(n) => Some(Constant::Number(*n)),
            Literal::Decimal(d) => Some(Constant::Decimal(*d)),
            Literal::String(s) => Some(Constant::String(s.clone())),
            Literal::Char(c) => Some(Constant::Char(*c)),
            Literal::FormattedString { .. } => unreachable!("handled above"),
            Literal::Bool(b) => Some(Constant::Bool(*b)),
            Literal::None => None,
            Literal::Unit => {
                let const_idx = self.program.add_constant(Constant::Unit);
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(const_idx)),
                ));
                return Ok(());
            }
            Literal::Timeframe(tf) => Some(Constant::Timeframe(*tf)),
        };

        if let Some(const_val) = const_val {
            let const_idx = self.program.add_constant(const_val);
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(const_idx)),
            ));
        } else {
            self.emit(Instruction::simple(OpCode::PushNull));
        }

        Ok(())
    }

    /// Compile binary operator
    // Superseded by the strict-typing typed dispatch (`compile_binary_op_inner` /
    // `helpers::emit_binary_op`); every arith/cmp/bitwise arm is `unreachable!`.
    // Retained as the documented post-strict-typing reference path.
    #[allow(dead_code)]
    pub(super) fn compile_binary_op(&mut self, op: &BinaryOp) -> Result<()> {
        let opcode = match op {
            BinaryOp::Add => {
                unreachable!("generic Add should be handled by helpers::emit_binary_op")
            }
            BinaryOp::Sub => unreachable!("generic Sub should be handled by typed dispatch"),
            BinaryOp::Mul => unreachable!("generic Mul should be handled by typed dispatch"),
            BinaryOp::Div => unreachable!("generic Div should be handled by typed dispatch"),
            BinaryOp::Mod => unreachable!("generic Mod should be handled by typed dispatch"),
            BinaryOp::Pow => unreachable!("generic Pow should be handled by typed dispatch"),
            // c5 Phase B (v0.3.3, 2026-05-28) — bitwise binary ops handled
            // exclusively by the strict-typing gate at
            // `compiler/expressions/binary_ops.rs:1403`. That arm emits
            // typed `BitAndInt`/`BitOrInt`/etc. on proven-int operands
            // and returns a compile error otherwise; the dynamic opcodes
            // (`BitAnd`/`BitOr`/`BitXor`/`BitShl`/`BitShr`) are deleted
            // along with their executor handlers.
            BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitShl
            | BinaryOp::BitXor
            | BinaryOp::BitShr => {
                unreachable!(
                    "bitwise binary ops handled by compile_binary_op_inner (typed-int gate)"
                )
            }
            BinaryOp::Greater => unreachable!("generic Gt should be handled by typed dispatch"),
            BinaryOp::Less => unreachable!("generic Lt should be handled by typed dispatch"),
            BinaryOp::GreaterEq => unreachable!("generic Gte should be handled by typed dispatch"),
            BinaryOp::LessEq => unreachable!("generic Lte should be handled by typed dispatch"),
            BinaryOp::Equal => unreachable!("generic Eq/Neq should be handled by typed dispatch"),
            BinaryOp::NotEqual => {
                unreachable!("generic Eq/Neq should be handled by typed dispatch")
            }
            BinaryOp::And => OpCode::And,
            BinaryOp::Or => OpCode::Or,
            BinaryOp::FuzzyEqual | BinaryOp::FuzzyGreater | BinaryOp::FuzzyLess => {
                // Fuzzy comparisons are desugared in compile_expr_fuzzy_comparison
                // and should never reach this path
                return Err(ShapeError::RuntimeError {
                    message: "Fuzzy comparison should be handled by compile_expr_fuzzy_comparison"
                        .to_string(),
                    location: None,
                });
            }
            BinaryOp::NullCoalesce => OpCode::NullCoalesce,
            BinaryOp::ErrorContext => OpCode::ErrorContext,
            BinaryOp::Pipe => {
                // Pipe is handled specially in compile_expr, should not reach here
                return Err(ShapeError::RuntimeError {
                    message: "Pipe operator should be handled specially".to_string(),
                    location: None,
                });
            }
        };

        self.emit(Instruction::simple(opcode));
        Ok(())
    }

    /// Compile unary operator
    pub(super) fn compile_unary_op(&mut self, op: &UnaryOp) -> Result<()> {
        let opcode = match op {
            UnaryOp::Not => OpCode::Not,
            UnaryOp::Neg => {
                unreachable!("generic Neg should be handled by unary_ops::compile_expr_unary_op")
            }
            // c5 Phase B (v0.3.3, 2026-05-28) — `~x` handled exclusively
            // by the strict-typing gate at `unary_ops.rs:28`. That arm
            // emits typed `BitNotInt` on a proven-int operand and returns
            // a compile error otherwise; the dynamic `BitNot` opcode is
            // deleted along with `exec_dyn_bit_unary`.
            UnaryOp::BitNot => unreachable!(
                "unary `~` handled by unary_ops::compile_expr_unary_op (typed-int gate)"
            ),
        };

        self.emit(Instruction::simple(opcode));
        Ok(())
    }
}
