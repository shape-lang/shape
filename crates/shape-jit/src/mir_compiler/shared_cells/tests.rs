use super::*;

#[test]
fn kind_source_disagreement_is_a_codegen_error() {
    let evidence = SharedLocalKindEvidence {
        layout: Some(NativeKind::Bool),
        inferred: Some(NativeKind::Float64),
        layout_conflict: None,
    };

    let error = evidence
        .validated(SlotId(7))
        .expect_err("disagreeing producer evidence must never select a kind");
    assert!(error.contains("kind-source disagreement"));
    assert!(error.contains("before cell allocation"));
}

#[test]
fn agreeing_sources_resolve_to_the_layout_kind() {
    let evidence = SharedLocalKindEvidence {
        layout: Some(NativeKind::Bool),
        inferred: Some(NativeKind::Bool),
        layout_conflict: None,
    };

    assert_eq!(evidence.validated(SlotId(7)), Ok(NativeKind::Bool));
}

#[test]
fn every_supported_ptr_kind_passes_shared_cell_abi_preflight() {
    for heap_kind in shape_value::HeapKind::ALL {
        let kind = NativeKind::Ptr(heap_kind);
        if heap_kind.has_kinded_slot_carrier() {
            let code = validated_shared_cell_kind_code(
                SlotId(7),
                kind,
                SharedCellCarrierOrigin::DeclaringFrameLocal,
            )
            .expect("every supported HeapKind carrier must preflight");
            assert_eq!(crate::ffi::stack_kind_code::decode(code), Some(kind));
        } else {
            let error = validated_shared_cell_kind_code(
                SlotId(7),
                kind,
                SharedCellCarrierOrigin::DeclaringFrameLocal,
            )
            .expect_err("carrier-less kinds must fail before JIT emission");
            assert!(error.contains("INTERNAL SharedCell kind invariant"));
            assert!(error.contains("Ptr(NativeScalar)"));
        }
    }
}

/// Lower-level D1 proof for the inherited-carrier branch.
///
/// ADR-009 C1 slice 4 now supplies the public source proof through the
/// generated and ordinary nested-share VM/JIT zero-fallback fixtures; they
/// mutate in the inner closure and observe through the outer holder. This
/// unit keeps the lower-level JIT carrier decision pinned: payload `42` is
/// not a valid Arc carrier and must never reach retain. Refcounted Shared
/// payloads and module-binding capture lowering remain separate W39 limits.
#[test]
fn inherited_shared_capture_retains_raw_cell_carrier_not_projected_payload() {
    use crate::ffi::object::{
        jit_alloc_shared_cell, jit_arc_shared_release, jit_arc_shared_retain,
    };

    let slot = SlotId(9);
    let operand = Operand::Copy(Place::Local(slot));
    let shared_local_slots = HashMap::new();
    let shared_capture_slots = HashMap::from([(slot, NativeKind::Int64)]);

    let lowering =
        classify_shared_capture_operand(&operand, &shared_local_slots, &shared_capture_slots);
    assert_eq!(
        lowering,
        SharedCaptureOperandLowering::RawCarrier {
            slot,
            origin: SharedCellCarrierOrigin::InheritedCapture,
        },
        "an inherited Shared parameter must bypass the lock-gated payload read"
    );

    let payload_bits = 42_u64;
    let cell_bits = unsafe {
        jit_alloc_shared_cell(
            payload_bits,
            crate::ffi::stack_kind_code::encode(NativeKind::Int64),
        )
    };
    assert_ne!(cell_bits, 0);
    assert_ne!(cell_bits, payload_bits);

    let selected_bits = match lowering {
        SharedCaptureOperandLowering::RawCarrier { .. } => cell_bits,
        SharedCaptureOperandLowering::ProjectedPayload => payload_bits,
    };
    let retained_bits = unsafe { jit_arc_shared_retain(selected_bits) };
    assert_eq!(
        retained_bits, cell_bits,
        "the nested closure must retain the cell identity, not payload bits"
    );

    unsafe {
        jit_arc_shared_release(retained_bits);
        jit_arc_shared_release(cell_bits);
    }
}
