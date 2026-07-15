use shape_ast::ast::Span;
use shape_ast::error::ShapeError;

use crate::compiler::BytecodeCompiler;

use super::{BindingKey, ReferenceFlowConflict};

impl BytecodeCompiler {
    pub(super) fn reference_flow_conflict_error(
        &self,
        conflict: ReferenceFlowConflict,
        fallback_span: Option<Span>,
    ) -> ShapeError {
        let binding_name = self
            .reference_flow_binding_name(conflict.key)
            .map(|name| format!(" (name '{name}')"))
            .unwrap_or_default();
        let location = self
            .reference_flow_binding_span(conflict.key)
            .filter(|span| !span.is_dummy())
            .or_else(|| fallback_span.filter(|span| !span.is_dummy()))
            .map(|span| self.span_to_source_location(span));
        let axis = if conflict.first.class == conflict.second.class {
            "storage class"
        } else {
            "reference representation or referent"
        };

        ShapeError::SemanticError {
            message: format!(
                "[C0912] exact reference-flow conflict at {} for {}{}: predecessor '{}' is {}, \
                 but predecessor '{}' is {}; reachable predecessors must agree on the exact \
                 reference class, referent evidence, and binding storage class ({axis} differs)",
                conflict.merge_name,
                conflict.key.description(),
                binding_name,
                conflict.first_label,
                conflict.first.description(),
                conflict.second_label,
                conflict.second.description(),
            ),
            location,
        }
    }

    fn reference_flow_binding_name(&self, key: BindingKey) -> Option<String> {
        let mut names: Vec<_> = match key {
            BindingKey::Local(slot) => self
                .locals
                .iter()
                .flat_map(|scope| scope.iter())
                .filter_map(|(name, &candidate)| (candidate == slot).then_some(name.clone()))
                .collect(),
            BindingKey::ModuleBinding(slot) => self
                .module_bindings
                .iter()
                .filter_map(|(name, &candidate)| (candidate == slot).then_some(name.clone()))
                .collect(),
        };
        names.sort();
        names.into_iter().next()
    }

    fn reference_flow_binding_span(&self, key: BindingKey) -> Option<Span> {
        match key {
            BindingKey::Local(slot) => self.local_binding_spans.get(&slot).copied(),
            BindingKey::ModuleBinding(slot) => self.module_binding_spans.get(&slot).copied(),
        }
    }
}
