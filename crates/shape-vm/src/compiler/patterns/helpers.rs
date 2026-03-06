//! Helper functions for pattern compilation

use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use shape_ast::error::Result;

use crate::compiler::BytecodeCompiler;

impl BytecodeCompiler {
    pub(super) fn emit_pattern_type_check(
        &mut self,
        value_local: u16,
        type_name: &str,
        fail_jumps: &mut Vec<usize>,
    ) -> Result<()> {
        let type_const = self.program.add_constant(Constant::TypeAnnotation(
            shape_ast::ast::TypeAnnotation::Basic(type_name.to_string()),
        ));
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(value_local)),
        ));
        self.emit(Instruction::new(
            OpCode::TypeCheck,
            Some(Operand::Const(type_const)),
        ));
        let jump = self.emit_jump(OpCode::JumpIfFalse, 0);
        fail_jumps.push(jump);
        Ok(())
    }

    pub(super) fn emit_object_rest(
        &mut self,
        excluded_keys: &[String],
        base_schema_id: Option<shape_runtime::type_schema::SchemaId>,
    ) -> Result<()> {
        if let Some(base_id) = base_schema_id {
            let mut excluded_sorted: Vec<&String> = excluded_keys.iter().collect();
            excluded_sorted.sort();
            let cache_name = format!(
                "__sub_{}_exc_{}",
                base_id,
                excluded_sorted
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            );

            if self
                .type_tracker
                .schema_registry()
                .get(&cache_name)
                .is_none()
            {
                let subset_fields = {
                    let registry = self.type_tracker.schema_registry();
                    registry.get_by_id(base_id).map(|base| {
                        base.fields
                            .iter()
                            .filter(|f| !excluded_keys.contains(&f.name))
                            .map(|f| (f.name.clone(), f.field_type.clone()))
                            .collect::<Vec<_>>()
                    })
                };
                if let Some(fields) = subset_fields {
                    self.type_tracker
                        .schema_registry_mut()
                        .register_type(cache_name, fields);
                }
            }
        }

        for key in excluded_keys {
            let key_const = self.program.add_constant(Constant::String(key.clone()));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(key_const)),
            ));
        }
        self.emit(Instruction::new(
            OpCode::NewArray,
            Some(Operand::Count(excluded_keys.len() as u16)),
        ));
        let arg_count = self.program.add_constant(Constant::Number(2.0));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(arg_count)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::ObjectRest)),
        ));
        Ok(())
    }

    pub(super) fn emit_destructure_type_check(
        &mut self,
        value_local: u16,
        type_name: &str,
        message: &str,
    ) -> Result<()> {
        let type_const = self.program.add_constant(Constant::TypeAnnotation(
            shape_ast::ast::TypeAnnotation::Basic(type_name.to_string()),
        ));
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(value_local)),
        ));
        self.emit(Instruction::new(
            OpCode::TypeCheck,
            Some(Operand::Const(type_const)),
        ));
        let ok_jump = self.emit_jump(OpCode::JumpIfTrue, 0);
        let msg = self
            .program
            .add_constant(Constant::String(message.to_string()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(msg)),
        ));
        self.emit(Instruction::simple(OpCode::Throw));
        self.patch_jump(ok_jump);
        Ok(())
    }
}
