use std::collections::BTreeSet;

use shape_ast::ast::Span;
use shape_ast::error::Result;

use crate::compiler::BytecodeCompiler;
use crate::type_tracking::BindingStorageClass;

use super::{
    BindingKey, ReferenceFlowConflict, ReferenceFlowEvidence, ReferenceFlowState,
};

impl BytecodeCompiler {
    /// Compile one callable under an exact reference-flow transaction.
    ///
    /// Local reference evidence is function-scoped. Module reference
    /// representation is visible to the callable but cannot change without an
    /// interprocedural effect summary, which C1 does not provide. The complete
    /// local binding-semantics scope stack and reference-flow state are restored
    /// on both `Ok` and `Err`; an existing compile error always wins over the
    /// success-only module transition check.
    pub(crate) fn with_callable_reference_flow_transaction<T>(
        &mut self,
        callable_name: &str,
        callable_span: Span,
        compile: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let saved_local_semantics = self.type_tracker.snapshot_local_binding_semantics();
        let saved_flow = self.enter_function_reference_flow();

        let outcome = compile(self);
        let transition_error = if outcome.is_ok() {
            let current_flow = self.reference_flow_snapshot();
            self.callable_module_transition_conflict(
                callable_name,
                &saved_flow,
                &current_flow,
            )
            .map(|conflict| {
                self.reference_flow_conflict_error(conflict, Some(callable_span))
            })
        } else {
            None
        };

        // Storage restoration updates existing BindingSemantics, so restore the
        // complete outer semantics authority before reinstalling exact flow.
        self.type_tracker
            .restore_local_binding_semantics(saved_local_semantics);
        self.restore_reference_flow_snapshot(&saved_flow);

        match (outcome, transition_error) {
            (Err(error), _) => Err(error),
            (Ok(_), Some(error)) => Err(error),
            (Ok(value), None) => Ok(value),
        }
    }

    fn callable_module_transition_conflict(
        &self,
        callable_name: &str,
        saved: &ReferenceFlowState,
        current: &ReferenceFlowState,
    ) -> Option<ReferenceFlowConflict> {
        let saved_keys = saved.keys();
        let current_keys = current.keys();
        let keys: BTreeSet<_> = saved_keys.union(&current_keys).copied().collect();

        keys.into_iter().find_map(|key| {
            if !matches!(key, BindingKey::ModuleBinding(_)) {
                return None;
            }
            let before = saved.evidence(key);
            let after = current.evidence(key);
            Self::module_reference_projection_changed(&before, &after).then(|| {
                ReferenceFlowConflict::new(
                    format!("callable '{callable_name}'"),
                    key,
                    "before callable",
                    before,
                    "after callable",
                    after,
                )
            })
        })
    }

    fn module_reference_projection_changed(
        before: &ReferenceFlowEvidence,
        after: &ReferenceFlowEvidence,
    ) -> bool {
        if before.class != after.class {
            return true;
        }

        let before_reference_storage =
            before.storage == Some(BindingStorageClass::Reference);
        let after_reference_storage = after.storage == Some(BindingStorageClass::Reference);
        if before_reference_storage != after_reference_storage {
            return true;
        }

        // Ordinary Value planning (for example Direct -> SharedCow) is not a
        // representation effect. Only a change in consistency between the
        // reference class and an explicitly known storage class is rejected.
        Self::reference_storage_is_consistent(before)
            != Self::reference_storage_is_consistent(after)
    }

    fn reference_storage_is_consistent(evidence: &ReferenceFlowEvidence) -> bool {
        match evidence.storage {
            None => true,
            Some(BindingStorageClass::Reference) => evidence.class.is_reference(),
            Some(_) => !evidence.class.is_reference(),
        }
    }
}
