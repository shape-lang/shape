//! Data access expression compilation

use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use shape_ast::ast::{DataIndex, Expr};
use shape_ast::error::{Result, ShapeError};

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a data reference expression (data[i])
    pub(super) fn compile_expr_data_ref(
        &mut self,
        data_ref: &shape_ast::ast::DataRef,
    ) -> Result<()> {
        // If timeframe is specified, push timeframe context
        if let Some(timeframe) = data_ref.timeframe {
            let tf_const = self.program.add_constant(Constant::Timeframe(timeframe));
            self.emit(Instruction::new(
                OpCode::PushTimeframe,
                Some(Operand::Const(tf_const)),
            ));
        }

        // Check if we have a schema - use optimized GetDataRow
        let has_schema = self.program.data_schema.is_some();

        match &data_ref.index {
            DataIndex::Single(idx) if has_schema => {
                // Optimized path: use GetDataRow for single row access
                let const_idx = self.program.add_constant(Constant::Number(*idx as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(const_idx)),
                ));
                self.emit(Instruction::simple(OpCode::GetDataRow));
            }
            DataIndex::Expression(expr) if has_schema => {
                // Optimized path: dynamic index with GetDataRow
                self.compile_expr(expr)?;
                self.emit(Instruction::simple(OpCode::GetDataRow));
            }
            _ => {
                // Explicit data variable path: requires 'data' to be explicitly declared in scope
                // Either as a function parameter or a local variable binding
                if let Some(local_idx) = self.resolve_local("data") {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(local_idx)),
                    ));
                } else {
                    // Provide clear guidance on how to fix this
                    return Err(ShapeError::SemanticError {
                        message: "data[...] requires explicit data binding. Either: \
                             (1) Set a DataSchema on the compiler for optimized access, \
                             (2) Pass 'data' as a function parameter, or \
                             (3) Bind 'data' with: let data = ..."
                            .to_string(),
                        location: None,
                    });
                }

                match &data_ref.index {
                    DataIndex::Single(idx) => {
                        // Static single index: data[0]
                        let const_idx = self.program.add_constant(Constant::Number(*idx as f64));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(const_idx)),
                        ));
                        self.emit(Instruction::simple(OpCode::GetProp));
                    }
                    DataIndex::Range(start, end) => {
                        // Static range: data[1:5]
                        let start_const =
                            self.program.add_constant(Constant::Number(*start as f64));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(start_const)),
                        ));
                        let end_const = self.program.add_constant(Constant::Number(*end as f64));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(end_const)),
                        ));
                        self.emit(Instruction::simple(OpCode::SliceAccess));
                    }
                    DataIndex::Expression(expr) => {
                        // Dynamic index: data[variable_name]
                        self.compile_expr(expr)?;
                        self.emit(Instruction::simple(OpCode::GetProp));
                    }
                    DataIndex::ExpressionRange(start, end) => {
                        // Dynamic range: data[start_expr:end_expr]
                        self.compile_expr(start)?;
                        self.compile_expr(end)?;
                        self.emit(Instruction::simple(OpCode::SliceAccess));
                    }
                }
            }
        }

        // If timeframe was specified, pop timeframe context
        if data_ref.timeframe.is_some() {
            self.emit(Instruction::simple(OpCode::PopTimeframe));
        }
        Ok(())
    }

    /// Compile a data datetime reference expression
    pub(super) fn compile_expr_data_datetime_ref(
        &mut self,
        datetime_ref: &shape_ast::ast::DataDateTimeRef,
    ) -> Result<()> {
        let const_idx = self
            .program
            .add_constant(Constant::DataDateTimeRef(datetime_ref.clone()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(const_idx)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::EvalDataDateTimeRef)),
        ));
        Ok(())
    }

    /// Compile a data relative access expression
    pub(super) fn compile_expr_data_relative_access(
        &mut self,
        reference: &Expr,
        index: &DataIndex,
    ) -> Result<()> {
        let is_range = matches!(
            index,
            DataIndex::Range(_, _) | DataIndex::ExpressionRange(_, _)
        );

        // Compile the reference expression (e.g., a series or data variable)
        self.compile_expr(reference)?;

        // Compile the index
        match index {
            DataIndex::Single(idx) => {
                let const_idx = self.program.add_constant(Constant::Number(*idx as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(const_idx)),
                ));
            }
            DataIndex::Range(start, end) => {
                let start_const = self.program.add_constant(Constant::Number(*start as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(start_const)),
                ));
                let end_const = self.program.add_constant(Constant::Number(*end as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(end_const)),
                ));
            }
            DataIndex::Expression(expr) => {
                self.compile_expr(expr)?;
            }
            DataIndex::ExpressionRange(start, end) => {
                self.compile_expr(start)?;
                self.compile_expr(end)?;
            }
        }

        // Use builtin function to evaluate relative access
        let builtin = if is_range {
            BuiltinFunction::EvalDataRelativeRange
        } else {
            BuiltinFunction::EvalDataRelative
        };
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(builtin)),
        ));
        Ok(())
    }
}
