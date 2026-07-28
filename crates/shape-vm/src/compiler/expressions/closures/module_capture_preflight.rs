//! Pre-emission refusal for callable-introduced module storage effects.

use shape_ast::ast::Span;
use shape_ast::error::{Result, ShapeError};

use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::capture_plan::{
    CaptureAccess, CaptureTarget, PlannedCapture,
};
use crate::type_tracking::BindingStorageClass;

impl BytecodeCompiler {
    /// Refuse a closure capture that would promote a module binding while an
    /// enclosing callable is being compiled.
    ///
    /// C1 has no interprocedural effect summary, so a callable cannot publish
    /// a new module representation. The canonical capture plan already owns
    /// the exact structural target, selected access discipline, storage fact,
    /// and promotion witness; this check deliberately performs no second AST
    /// or name-based classification pass.
    pub(super) fn preflight_callable_module_shared_captures(
        &self,
        plan: &[PlannedCapture],
        closure_span: Span,
    ) -> Result<()> {
        let Some(current_function) = self.current_function else {
            return Ok(());
        };
        for planned in plan {
            if !planned.plan.needs_cell() {
                continue;
            }
            let Some(CaptureTarget::ModuleBinding(slot)) = planned.facts.target else {
                continue;
            };
            let already_published = planned.plan.access() == CaptureAccess::SharedCell
                && planned.facts.storage == Some(BindingStorageClass::SharedCow)
                && planned.facts.witness_shared_module_binding;
            if already_published {
                continue;
            }
            let callable_name = self
                .program
                .functions
                .get(current_function)
                .map(|function| function.name.clone())
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: format!(
                        "internal compiler error: active callable index {current_function} has no \
                         function metadata during closure-capture preflight"
                    ),
                    location: None,
                })?;

            return Err(self.module_capture_storage_effect_conflict(
                &callable_name,
                slot,
                planned.facts.witness_shared_module_binding,
                closure_span,
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "module_capture_preflight/tests.rs"]
mod tests;
