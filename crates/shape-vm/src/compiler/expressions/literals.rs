//! Literal expression compilation

use crate::type_tracking::VariableTypeInfo;
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
            _ => None,
        };
        // Track numeric type for typed opcode emission. A char literal IS its
        // integer code point (operators.mdx "Character Literals") — track it as
        // a plain `int` so `'A' + 1`, `let c: int = 'A'`, and `arr['A' - 'A']`
        // all flow through int typed-opcode dispatch. No distinct char type.
        Ok(())
    }
}
