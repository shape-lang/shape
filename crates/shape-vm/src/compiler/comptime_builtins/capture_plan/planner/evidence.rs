//! Structural inference and inherited-pack evidence for capture planning.

use super::*;
use crate::compiler::BytecodeCompiler;

impl BytecodeCompiler {
    pub(super) fn capture_binding_facts(
        &self,
        name: &str,
        mutated: bool,
        origin: Option<&GeneratedNodeOrigin>,
        ordinal: u16,
        freeze: std::result::Result<&super::super::super::semantic_freeze::FreezeOverlay, String>,
    ) -> CaptureBindingFacts {
        let target = self.resolve_capture_target(name);
        let inherited_evidence = match target {
            Some(CaptureTarget::Local(idx)) => self.inherited_capture_parameter_evidence.get(&idx),
            _ => None,
        };
        let storage = match (target, inherited_evidence) {
            (Some(CaptureTarget::Local(_)), Some(evidence))
                if evidence.access == CaptureAccess::SharedCell =>
            {
                Some(BindingStorageClass::SharedCow)
            }
            (Some(CaptureTarget::Local(idx)), _) => self.mir_storage_class_for_slot(idx),
            _ => None,
        };

        let inherited_shared_cell =
            inherited_evidence.is_some_and(|evidence| evidence.access == CaptureAccess::SharedCell);
        let binding_span = match target {
            Some(CaptureTarget::Local(idx)) => self.local_binding_spans.get(&idx).copied(),
            Some(CaptureTarget::ModuleBinding(idx)) => self.module_binding_spans.get(&idx).copied(),
            None => None,
        };
        let source_semantic_type =
            self.generated_capture_semantic_evidence(name, origin, ordinal, freeze);
        let semantic_type = match inherited_evidence {
            None => source_semantic_type,
            Some(inherited) if inherited.semantic_type == source_semantic_type => {
                source_semantic_type
            }
            Some(inherited) => CaptureSemanticEvidence::conflict(
                CaptureSemanticIssueKind::InferenceConflict,
                format!(
                    "capture '{name}' structural source fact disagrees with inherited pack evidence: source={source_semantic_type:?}, inherited={:?}",
                    inherited.semantic_type
                ),
            ),
        };

        CaptureBindingFacts {
            name: name.to_string(),
            target,
            binding_span,
            binding_lineage: inherited_evidence
                .and_then(|evidence| evidence.binding_lineage.clone()),
            binding_file_id: self.current_file_id,
            semantic_type,
            ownership: self
                .binding_semantics_for_name(name)
                .map(|(_, _, sem)| sem.ownership_class),
            storage,
            mutated,
            boxed: self.boxed_locals.contains(name),
            witness_shared_local: self.shared_locals.contains(name),
            witness_shared_module_binding: self.shared_module_binding_contains(name),
            witness_owned_mutable_local: self.owned_mutable_locals.contains(name),
            inherited_capture_parameter: inherited_evidence.is_some(),
            inherited_shared_cell,
        }
    }

    fn generated_capture_semantic_evidence(
        &self,
        name: &str,
        origin: Option<&GeneratedNodeOrigin>,
        ordinal: u16,
        freeze: std::result::Result<&super::super::super::semantic_freeze::FreezeOverlay, String>,
    ) -> CaptureSemanticEvidence {
        let Some(origin) = origin else {
            return CaptureSemanticEvidence::unavailable(
                CaptureSemanticIssueKind::MissingInferenceFact,
                format!("capture '{name}' belongs to ordinary source"),
            );
        };
        let key = shape_runtime::type_system::GeneratedCaptureKey::new(
            shape_runtime::type_system::GeneratedNodeKey::from_origin(origin),
            ordinal,
        );
        let Some(fact) = self.inference_facts.generated_capture_fact(&key) else {
            return CaptureSemanticEvidence::unavailable(
                CaptureSemanticIssueKind::MissingInferenceFact,
                format!("capture '{name}' has no structural inference fact at ordinal {ordinal}"),
            );
        };
        let candidate = match fact {
            shape_runtime::type_system::GeneratedCaptureFact::Exact(candidate) => candidate,
            shape_runtime::type_system::GeneratedCaptureFact::Unavailable(issue) => {
                return CaptureSemanticEvidence::unavailable(
                    CaptureSemanticIssueKind::InferenceUnavailable,
                    format!(
                        "capture '{name}' structural inference is unavailable: {}",
                        issue.detail()
                    ),
                );
            }
            shape_runtime::type_system::GeneratedCaptureFact::Conflict(issue) => {
                return CaptureSemanticEvidence::conflict(
                    CaptureSemanticIssueKind::InferenceConflict,
                    format!(
                        "capture '{name}' structural inference conflicts: {}",
                        issue.detail()
                    ),
                );
            }
        };
        let freeze = match freeze {
            Ok(freeze) => freeze,
            Err(detail) => {
                return CaptureSemanticEvidence::unavailable(
                    CaptureSemanticIssueKind::MissingSemanticFreeze,
                    format!("capture '{name}' has no semantic-freeze projection: {detail}"),
                );
            }
        };
        match CaptureSemanticType::from_semantic_candidate(candidate, freeze) {
            Ok(semantic_type) => CaptureSemanticEvidence::Exact(semantic_type),
            Err(detail) => CaptureSemanticEvidence::unavailable(
                CaptureSemanticIssueKind::FreezeRejected,
                format!("capture '{name}' is not semantically exact: {detail}"),
            ),
        }
    }
}
