//! Additional control-flow helpers kept separate for file-size maintainability.

use cranelift::prelude::*;

use crate::nan_boxing::*;
use shape_vm::bytecode::{Constant, Instruction, OpCode, Operand};

use crate::translator::types::BytecodeToIR;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Jump to a known block target, honoring synthetic merge-block stack params.
    pub(in crate::translator::opcodes) fn emit_jump_to_target(
        &mut self,
        target_idx: usize,
        target_block: Block,
    ) {
        if self.merge_blocks.contains(&target_idx) {
            let val = self
                .stack_pop()
                .unwrap_or_else(|| self.builder.ins().iconst(types::I64, TAG_NULL as i64));
            self.builder.ins().jump(target_block, &[val]);
        } else {
            self.builder.ins().jump(target_block, &[]);
        }
    }

    // Iterator operations.
    pub(crate) fn compile_iter_next(&mut self) -> Result<(), String> {
        if self.stack_len() >= 2 {
            let idx_val = self.stack_pop().unwrap();
            let iter = self.stack_pop().unwrap();
            let inst = self
                .builder
                .ins()
                .call(self.ffi.iter_next, &[iter, idx_val]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push(result);
        } else {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        }
        Ok(())
    }

    pub(crate) fn compile_iter_done(&mut self) -> Result<(), String> {
        if self.stack_len() >= 2 {
            let idx = self.stack_pop().unwrap();
            let iter = self.stack_pop().unwrap();
            let inst = self.builder.ins().call(self.ffi.iter_done, &[iter, idx]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push(result);
        } else {
            let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
            self.stack_push(true_val);
        }
        Ok(())
    }

    /// Type check: pops value, checks if it matches type annotation, pushes boolean.
    pub(crate) fn compile_type_check(&mut self, instr: &Instruction) -> Result<(), String> {
        fn encode_type(type_ann: &crate::ast::TypeAnnotation) -> String {
            match type_ann {
                crate::ast::TypeAnnotation::Basic(name) => format!("basic:{name}"),
                crate::ast::TypeAnnotation::Array(inner) => {
                    format!("array:{}", encode_type(inner))
                }
                crate::ast::TypeAnnotation::Function { .. } => "function".to_string(),
                crate::ast::TypeAnnotation::Optional(inner) => {
                    format!("optional:{}", encode_type(inner))
                }
                crate::ast::TypeAnnotation::Tuple(types) => {
                    let inner: Vec<String> = types.iter().map(encode_type).collect();
                    format!("tuple:{}", inner.join(","))
                }
                crate::ast::TypeAnnotation::Object(_) => "object".to_string(),
                crate::ast::TypeAnnotation::Union(_) => "unknown".to_string(),
                crate::ast::TypeAnnotation::Intersection(_) => "unknown".to_string(),
                crate::ast::TypeAnnotation::Generic { name, .. } => format!("generic:{name}"),
                crate::ast::TypeAnnotation::Any => "any".to_string(),
                crate::ast::TypeAnnotation::Void => "void".to_string(),
                crate::ast::TypeAnnotation::Never => "never".to_string(),
                crate::ast::TypeAnnotation::Null => "null".to_string(),
                crate::ast::TypeAnnotation::Undefined => "undefined".to_string(),
                crate::ast::TypeAnnotation::Reference(name) => format!("ref:{name}"),
                crate::ast::TypeAnnotation::Dyn(traits) => format!("dyn:{}", traits.join("+")),
            }
        }

        let type_name = if let Some(Operand::Const(const_idx)) = &instr.operand {
            match &self.program.constants[*const_idx as usize] {
                Constant::TypeAnnotation(type_ann) => encode_type(type_ann),
                Constant::String(s) => format!("basic:{s}"),
                _ => "unknown".to_string(),
            }
        } else {
            "unknown".to_string()
        };

        if let Some(value) = self.stack_pop() {
            let type_name_bits = jit_box(HK_STRING, type_name);
            let type_name_val = self.builder.ins().iconst(types::I64, type_name_bits as i64);
            let inst = self
                .builder
                .ins()
                .call(self.ffi.type_check, &[value, type_name_val]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push(result);
        } else {
            let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
            self.stack_push(false_val);
        }
        Ok(())
    }

    /// TryUnwrap: unwrap Ok value or early-return Err (`?` operator lowering).
    pub(crate) fn compile_try_unwrap(&mut self) -> Result<(), String> {
        if let Some(value) = self.stack_pop() {
            let inst = self.builder.ins().call(self.ffi.is_ok, &[value]);
            let is_ok_result = self.builder.inst_results(inst)[0];
            let true_tag = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
            let is_ok = self
                .builder
                .ins()
                .icmp(IntCC::Equal, is_ok_result, true_tag);

            let ok_block = self.builder.create_block();
            let err_block = self.builder.create_block();
            let continue_block = self.builder.create_block();
            self.builder.append_block_param(continue_block, types::I64);
            self.builder
                .ins()
                .brif(is_ok, ok_block, &[], err_block, &[]);

            self.builder.switch_to_block(ok_block);
            self.builder.seal_block(ok_block);
            let unwrap_inst = self.builder.ins().call(self.ffi.unwrap_ok, &[value]);
            let inner_value = self.builder.inst_results(unwrap_inst)[0];
            self.builder.ins().jump(continue_block, &[inner_value]);

            self.builder.switch_to_block(err_block);
            self.builder.seal_block(err_block);
            if let Some(exit_block) = self.exit_block {
                self.builder.ins().jump(exit_block, &[value]);
            } else {
                let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                self.builder.ins().jump(continue_block, &[null_val]);
            }

            self.builder.switch_to_block(continue_block);
            self.builder.seal_block(continue_block);
            let inner_val = self.builder.block_params(continue_block)[0];
            self.stack_push(inner_val);
        } else {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        }
        Ok(())
    }

    /// Promote boxed module bindings to loop-carried registers (no type conversion).
    pub(in crate::translator::opcodes) fn promote_register_carried_module_bindings(
        &mut self,
        info: &crate::translator::loop_analysis::LoopInfo,
    ) {
        if !self.register_carried_module_bindings.is_empty() {
            return;
        }
        let Some(loop_plan) = self.optimization_plan.loops.get(&info.header_idx) else {
            return;
        };
        if loop_plan.register_carried_module_bindings.is_empty() {
            return;
        }
        let has_nested_loops = ((info.header_idx + 1)..info.end_idx)
            .any(|i| self.program.instructions[i].opcode == OpCode::LoopStart);
        if self.loop_stack.len() > 1 || has_nested_loops {
            return;
        }

        let has_calls = ((info.header_idx + 1)..info.end_idx).any(|i| {
            matches!(
                self.program.instructions[i].opcode,
                OpCode::Call
                    | OpCode::CallValue
                    | OpCode::CallMethod
                    | OpCode::DynMethodCall
                    | OpCode::BuiltinCall
            )
        });
        if has_calls {
            return;
        }

        let mut any_promoted = false;
        let mut bindings: Vec<u16> = loop_plan
            .register_carried_module_bindings
            .iter()
            .copied()
            .collect();
        bindings.sort_unstable();
        for mb_idx in bindings {
            if self.unboxed_int_module_bindings.contains(&mb_idx) {
                continue;
            }
            let var = self.get_or_create_module_binding_var(mb_idx);
            let byte_offset = crate::context::LOCALS_OFFSET + (mb_idx as i32 * 8);
            let boxed =
                self.builder
                    .ins()
                    .load(types::I64, MemFlags::new(), self.ctx_ptr, byte_offset);
            self.builder.def_var(var, boxed);
            self.register_carried_module_bindings.insert(mb_idx);
            any_promoted = true;
        }
        if any_promoted {
            self.register_carried_loop_depth = self.loop_stack.len();
        }
    }

    pub(in crate::translator::opcodes) fn get_or_create_module_binding_var(
        &mut self,
        mb_idx: u16,
    ) -> Variable {
        if let Some(&var) = self.promoted_module_bindings.get(&mb_idx) {
            return var;
        }
        let var = Variable::new(self.next_var);
        self.next_var += 1;
        self.builder.declare_var(var, types::I64);
        self.promoted_module_bindings.insert(mb_idx, var);
        var
    }
}
