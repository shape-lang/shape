//! Closure (function expression) compilation

use crate::bytecode::{Function, Instruction, OpCode, Operand};
use shape_ast::ast::{Expr, FunctionDef, Span};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::closure::EnvironmentAnalyzer;
use std::collections::BTreeSet;

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a function expression (closure)
    pub(super) fn compile_expr_closure(
        &mut self,
        params: &[shape_ast::ast::FunctionParameter],
        body: &[shape_ast::ast::Statement],
    ) -> Result<()> {
        let closure_name = format!("__closure_{}", self.closure_counter);
        self.closure_counter += 1;

        let proto_def = FunctionDef {
            name: closure_name.clone(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: params.to_vec(),
            return_type: None,
            body: body.to_vec(),
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        let outer_vars = self.collect_outer_scope_vars();
        let (mut captured_vars, mutated_captures) =
            EnvironmentAnalyzer::analyze_function_with_mutability(&proto_def, &outer_vars);
        captured_vars.sort();
        let param_names: BTreeSet<String> =
            params.iter().flat_map(|p| p.get_identifiers()).collect();
        captured_vars.retain(|name| !param_names.contains(name));

        // Shape references are call-scoped; we do not allow closure capture of
        // reference-typed locals because that would permit escaping borrows.
        for captured in &captured_vars {
            if let Some(local_idx) = self.resolve_local(captured)
                && self.ref_locals.contains(&local_idx)
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "[B0003] reference '{}' cannot escape into a closure; capture a value instead",
                        captured
                    ),
                    location: None,
                });
            }
        }

        // Build per-capture mutability flags (aligned with captured_vars order).
        // A capture is mutable if the closure itself mutates it OR if a previous
        // closure in the same scope already boxed it into a SharedCell.
        let mutable_flags: Vec<bool> = captured_vars
            .iter()
            .map(|name| mutated_captures.contains(name) || self.boxed_locals.contains(name))
            .collect();

        // Build closure parameters: only immutable captures become leading params.
        // Mutable captures are accessed via LoadClosure/StoreClosure opcodes.
        let mut closure_params = Vec::with_capacity(captured_vars.len() + params.len());
        for name in &captured_vars {
            closure_params.push(shape_ast::ast::FunctionParameter {
                pattern: shape_ast::ast::DestructurePattern::Identifier(name.clone(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: None,
                default_value: None,
            });
        }
        closure_params.extend(params.to_vec());

        let closure_def = FunctionDef {
            name: closure_name.clone(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: closure_params,
            return_type: None,
            body: body.to_vec(),
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        let func_idx = self.program.functions.len();
        self.program.functions.push(Function {
            name: closure_name.clone(),
            arity: closure_def.params.len() as u16,
            param_names: closure_def
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect(),
            locals_count: 0,
            entry_point: 0,
            body_length: 0,
            is_closure: true,
            captures_count: captured_vars.len() as u16,
            is_async: false,
            ref_params: Vec::new(),
            ref_mutates: Vec::new(),
            mutable_captures: mutable_flags.clone(),
            frame_descriptor: None,
            osr_entry_points: Vec::new(),
        });

        // Set up mutable_closure_captures so that during body compilation,
        // variable accesses for mutable captures emit LoadClosure/StoreClosure.
        let saved_mutable_captures = std::mem::take(&mut self.mutable_closure_captures);
        for (i, name) in captured_vars.iter().enumerate() {
            if mutable_flags.get(i).copied().unwrap_or(false) {
                self.mutable_closure_captures.insert(name.clone(), i as u16);
            }
        }

        let jump_over = self.emit_jump(OpCode::Jump, 0);
        self.compile_function(&closure_def)?;
        self.patch_jump(jump_over);

        // Restore mutable_closure_captures
        self.mutable_closure_captures = saved_mutable_captures;

        for (i, captured) in captured_vars.iter().enumerate() {
            if mutable_flags.get(i).copied().unwrap_or(false) {
                // Mutable capture: emit BoxLocal/BoxModuleBinding to convert the
                // variable to a SharedCell and push the cell onto the stack.
                // MakeClosure will extract the Arc so the closure and enclosing
                // scope share the same mutable cell.
                // Track that this variable has been boxed so subsequent closures
                // in the same scope also use the SharedCell path.
                self.boxed_locals.insert(captured.clone());
                if let Some(local_idx) = self.resolve_local(captured) {
                    self.emit(Instruction::new(
                        OpCode::BoxLocal,
                        Some(Operand::Local(local_idx)),
                    ));
                } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(captured)
                {
                    let mb_idx = self.get_or_create_module_binding(&scoped_name);
                    self.emit(Instruction::new(
                        OpCode::BoxModuleBinding,
                        Some(Operand::ModuleBinding(mb_idx)),
                    ));
                } else if self.module_bindings.contains_key(captured) {
                    let mb_idx = self.get_or_create_module_binding(captured);
                    self.emit(Instruction::new(
                        OpCode::BoxModuleBinding,
                        Some(Operand::ModuleBinding(mb_idx)),
                    ));
                } else {
                    // Last resort fallback — just load the value
                    let temp = Expr::Identifier(captured.clone(), Span::DUMMY);
                    self.compile_expr(&temp)?;
                }
            } else {
                let temp = Expr::Identifier(captured.clone(), Span::DUMMY);
                self.compile_expr(&temp)?;
            }
        }

        self.emit(Instruction::new(
            OpCode::MakeClosure,
            Some(Operand::Function(shape_value::FunctionId(func_idx as u16))),
        ));
        // Closures don't produce TypedObjects
        self.last_expr_schema = None;
        Ok(())
    }
}
