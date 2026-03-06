//! Pattern reference expression compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use shape_ast::error::Result;
use shape_runtime::type_schema::FieldType;

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a pattern reference expression
    pub(super) fn compile_expr_pattern_ref(&mut self, name: &str) -> Result<()> {
        // Register schema for pattern ref object: { __type, name }
        let schema_id = self.type_tracker.register_inline_object_schema_typed(&[
            ("__type", FieldType::String),
            ("name", FieldType::String),
        ]);

        // Push values only (no key push — schema has field names)
        let type_value = self
            .program
            .add_constant(Constant::String("pattern_ref".to_string()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(type_value)),
        ));
        let name_value = self
            .program
            .add_constant(Constant::String(name.to_string()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(name_value)),
        ));

        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: schema_id as u16,
                field_count: 2,
            }),
        ));
        Ok(())
    }
}
