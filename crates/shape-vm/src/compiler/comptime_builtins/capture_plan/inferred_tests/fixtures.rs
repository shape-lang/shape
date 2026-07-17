use super::*;

pub(super) fn legacy_pair(f: &CaptureBindingFacts) -> (bool, CaptureKind) {
    let is_local_slot = f.is_local();
    let is_module_binding_slot = f.is_module_binding();

    // closures.rs:3236-3256
    let is_flexible_capture = matches!(f.ownership, Some(BindingOwnershipClass::Flexible))
        && (is_local_slot || is_module_binding_slot);
    let mutable_flag = f.mutated
        || f.boxed
        || f.witness_shared_local
        || f.witness_shared_module_binding
        || is_flexible_capture;

    // closures.rs:3550-3635
    let kind = if !mutable_flag {
        CaptureKind::Immutable
    } else {
        match f.ownership {
            Some(BindingOwnershipClass::OwnedMutable) if is_local_slot => CaptureKind::OwnedMutable,
            Some(BindingOwnershipClass::OwnedMutable) if is_module_binding_slot => {
                CaptureKind::Shared
            }
            Some(BindingOwnershipClass::OwnedMutable) => CaptureKind::Immutable,
            Some(BindingOwnershipClass::Flexible) if is_local_slot || is_module_binding_slot => {
                CaptureKind::Shared
            }
            Some(BindingOwnershipClass::Flexible) => CaptureKind::Immutable,
            _ if is_local_slot && f.witness_shared_local => CaptureKind::Shared,
            _ if is_local_slot && f.witness_owned_mutable_local => CaptureKind::OwnedMutable,
            _ if is_module_binding_slot && f.witness_shared_module_binding => CaptureKind::Shared,
            _ if is_module_binding_slot => CaptureKind::Shared,
            _ => CaptureKind::Immutable,
        }
    };
    (mutable_flag, kind)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn facts_for(
    target: Option<CaptureTarget>,
    ownership: Option<BindingOwnershipClass>,
    mutated: bool,
    boxed: bool,
    witness_shared_local: bool,
    witness_shared_module_binding: bool,
    witness_owned_mutable_local: bool,
) -> CaptureBindingFacts {
    CaptureBindingFacts {
        name: "x".to_string(),
        target,
        binding_span: None,
        binding_lineage: None,
        binding_file_id: 0,
        semantic_type: CaptureSemanticEvidence::unavailable(
            CaptureSemanticIssueKind::MissingInferenceFact,
            "selector test has no binding inference subject",
        ),
        ownership,
        storage: None,
        mutated,
        boxed,
        witness_shared_local,
        witness_shared_module_binding,
        witness_owned_mutable_local,
        inherited_capture_parameter: false,
        inherited_shared_cell: false,
    }
}
