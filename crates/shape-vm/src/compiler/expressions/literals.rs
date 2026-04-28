//! Literal expression compilation

use crate::type_tracking::{NumericType, VariableTypeInfo};
use shape_ast::ast::Literal;
use shape_ast::error::Result;

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a literal expression
    pub(super) fn compile_expr_literal(&mut self, lit: &Literal) -> Result<()> {
        self.compile_literal(lit)?;
        // Literals don't produce TypedObjects
        self.last_expr_schema = None;
        // Phase 3e: propagate non-numeric primitive literal types
        // (string, bool, char) so `let mut s = ""` records s as
        // string in the tracker. Numeric literals continue to use
        // last_expr_numeric_type for typed-opcode emission.
        self.last_expr_type_info = match lit {
            Literal::String(_) => Some(VariableTypeInfo::named("string".to_string())),
            Literal::Bool(_) => Some(VariableTypeInfo::named("bool".to_string())),
            Literal::Char(_) => Some(VariableTypeInfo::named("char".to_string())),
            _ => None,
        };
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
