//! Literal expression compilation

use crate::type_tracking::NumericType;
use shape_ast::ast::Literal;
use shape_ast::error::Result;

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a literal expression
    pub(super) fn compile_expr_literal(&mut self, lit: &Literal) -> Result<()> {
        self.compile_literal(lit)?;
        // Literals don't produce TypedObjects
        self.last_expr_schema = None;
        self.last_expr_type_info = None;
        // Track numeric type for typed opcode emission
        self.last_expr_numeric_type = match lit {
            Literal::Int(_) => Some(NumericType::Int),
            Literal::UInt(_) => Some(NumericType::IntWidth(shape_ast::IntWidth::U64)),
            Literal::TypedInt(_, w) => Some(NumericType::IntWidth(*w)),
            Literal::Number(_) => Some(NumericType::Number),
            Literal::Decimal(_) => Some(NumericType::Decimal),
            _ => None,
        };
        Ok(())
    }
}
