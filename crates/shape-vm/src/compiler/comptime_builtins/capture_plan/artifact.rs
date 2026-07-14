use crate::bytecode::OpCode;

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

pub(super) fn family_for_access(access: CaptureAccess) -> Option<CellCaptureFamily> {
    match access {
        CaptureAccess::Param => None,
        CaptureAccess::OwnedMutableCell => Some(CellCaptureFamily::OwnedMutable),
        CaptureAccess::SharedCell => Some(CellCaptureFamily::Shared),
        CaptureAccess::MutableCell => Some(CellCaptureFamily::Legacy),
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
    }
}
