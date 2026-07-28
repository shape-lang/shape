use crate::bytecode::OpCode;
use crate::compiler::helpers::{
    owned_mutable_typed_load_opcode, owned_mutable_typed_store_opcode, shared_typed_load_opcode,
    shared_typed_store_opcode,
};
use shape_value::v2::struct_layout::FieldKind;

use super::CaptureAccess;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CellCaptureFamily {
    Legacy,
    OwnedMutable,
    Shared,
}

/// Classify the complete capture-cell opcode ranges. Their discriminants are
/// pinned and contiguous in `opcode_defs.rs`; ordinary local loads/stores do
/// not enter this classifier.
pub(super) fn cell_capture_family(opcode: OpCode) -> Option<CellCaptureFamily> {
    match opcode as u16 {
        0x54 | 0x55 => Some(CellCaptureFamily::Legacy),
        0x132 | 0x133 | 0x140..=0x155 => Some(CellCaptureFamily::OwnedMutable),
        0x134 | 0x135 | 0x156..=0x16B => Some(CellCaptureFamily::Shared),
        _ => None,
    }
}

/// The only capture-cell opcodes an emitted descriptor may use.
///
/// Shared and uniquely owned cells are always typed from the capture pack's
/// resolved payload kind. The legacy dynamic pair remains valid only for the
/// inference residual represented explicitly by [`CaptureAccess::MutableCell`].
pub(super) fn exact_opcodes_for_access(
    access: CaptureAccess,
    payload_kind: FieldKind,
) -> Option<[OpCode; 2]> {
    match access {
        CaptureAccess::Param => None,
        CaptureAccess::MutableCell => Some([OpCode::LoadClosure, OpCode::StoreClosure]),
        CaptureAccess::OwnedMutableCell => Some([
            owned_mutable_typed_load_opcode(payload_kind),
            owned_mutable_typed_store_opcode(payload_kind),
        ]),
        CaptureAccess::SharedCell => Some([
            shared_typed_load_opcode(payload_kind),
            shared_typed_store_opcode(payload_kind),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_opcode_boundaries_are_closed() {
        assert_eq!(
            cell_capture_family(OpCode::LoadClosure),
            Some(CellCaptureFamily::Legacy)
        );
        assert_eq!(
            cell_capture_family(OpCode::StoreOwnedMutableCapturePtr),
            Some(CellCaptureFamily::OwnedMutable)
        );
        assert_eq!(
            cell_capture_family(OpCode::StoreSharedCapturePtr),
            Some(CellCaptureFamily::Shared)
        );
        assert_eq!(cell_capture_family(OpCode::LoadLocal), None);
        assert_eq!(
            exact_opcodes_for_access(CaptureAccess::MutableCell, FieldKind::I64),
            Some([OpCode::LoadClosure, OpCode::StoreClosure]),
        );
        assert_eq!(
            exact_opcodes_for_access(CaptureAccess::OwnedMutableCell, FieldKind::I64),
            Some([
                OpCode::LoadOwnedMutableCaptureI64,
                OpCode::StoreOwnedMutableCaptureI64,
            ]),
        );
        assert_eq!(
            exact_opcodes_for_access(CaptureAccess::SharedCell, FieldKind::U64),
            Some([OpCode::LoadSharedCaptureU64, OpCode::StoreSharedCaptureU64,]),
        );
    }
}
