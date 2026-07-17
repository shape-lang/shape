//! Canonical `ConcreteType -> NativeKind` authority for closure captures.
//!
//! Both ordinary and generated capture packs reach `ClosureRegistry` through
//! this exhaustive mapping. It therefore also proves that the semantic issuer
//! cannot publish the carrier-less `Ptr(HeapKind::NativeScalar)` kind.

use super::concrete_type::ConcreteType;
use crate::{HeapKind, NativeKind};

/// Map a `ConcreteType` to the matching `NativeKind` for closure-capture kind
/// tracking (ADR-006 §2.7.8 / Q10).
///
/// The mapping is total and post-proof per §2.7.5.1. There is no unknown,
/// dynamic, or Bool-default arm. `NativeScalar` deliberately has no
/// `ConcreteType`: it is a legacy width-preserving enum with no chosen 8-byte
/// kinded carrier, so no source or generated capture may issue that kind.
pub fn native_kind_from_concrete_type(ty: &ConcreteType) -> NativeKind {
    let kind = match ty {
        ConcreteType::F64 => NativeKind::Float64,
        ConcreteType::I64 => NativeKind::Int64,
        ConcreteType::I32 => NativeKind::Int32,
        ConcreteType::I16 => NativeKind::Int16,
        ConcreteType::I8 => NativeKind::Int8,
        ConcreteType::U64 => NativeKind::UInt64,
        ConcreteType::U32 => NativeKind::UInt32,
        ConcreteType::U16 => NativeKind::UInt16,
        ConcreteType::U8 => NativeKind::UInt8,
        ConcreteType::Bool => NativeKind::Bool,
        ConcreteType::String => NativeKind::String,
        ConcreteType::Array(_) => NativeKind::Ptr(HeapKind::TypedArray),
        ConcreteType::HashMap(_, _) => NativeKind::Ptr(HeapKind::HashMap),
        ConcreteType::Struct(_) | ConcreteType::Enum(_) | ConcreteType::Tuple(_) => {
            NativeKind::Ptr(HeapKind::TypedObject)
        }
        ConcreteType::Closure(_) | ConcreteType::Function(_) => NativeKind::Ptr(HeapKind::Closure),
        ConcreteType::Pointer(_) => NativeKind::Ptr(HeapKind::NativeView),
        ConcreteType::Decimal => NativeKind::Ptr(HeapKind::Decimal),
        ConcreteType::BigInt => NativeKind::Ptr(HeapKind::BigInt),
        ConcreteType::DateTime => NativeKind::Ptr(HeapKind::Temporal),
        ConcreteType::Option(_) | ConcreteType::Result(_, _) => {
            NativeKind::Ptr(HeapKind::TypedObject)
        }
        ConcreteType::HashSet(_) => NativeKind::Ptr(HeapKind::HashSet),
        ConcreteType::Deque(_) => NativeKind::Ptr(HeapKind::Deque),
        ConcreteType::PriorityQueue => NativeKind::Ptr(HeapKind::PriorityQueue),
        ConcreteType::Channel(_) => NativeKind::Ptr(HeapKind::Channel),
        ConcreteType::Mutex(_) => NativeKind::Ptr(HeapKind::Mutex),
        ConcreteType::Atomic => NativeKind::Ptr(HeapKind::Atomic),
        ConcreteType::Lazy(_) => NativeKind::Ptr(HeapKind::Lazy),
        ConcreteType::F32 => NativeKind::Float32,
        ConcreteType::Char => NativeKind::Char,
        ConcreteType::Void => panic!(
            "ClosureLayout: ConcreteType::Void is not a well-formed capture type \
             (ADR-006 §2.7.8 / Q10 — kinds must be concrete at construction; \
             no Bool-default fallback)"
        ),
    };

    assert!(
        !matches!(kind, NativeKind::Ptr(heap_kind) if !heap_kind.has_kinded_slot_carrier()),
        "internal compiler error: ConcreteType {ty:?} issued unsupported capture kind \
         {kind:?}; no closure layout or capture plan may publish a kind without a \
         nonzero 8-byte KindedSlot carrier"
    );
    kind
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::concrete_type::{
        ClosureTypeId, EnumLayoutId, FunctionTypeId, NamedTypeId, StructLayoutId,
    };

    fn representative_concrete_types() -> Vec<ConcreteType> {
        vec![
            ConcreteType::F64,
            ConcreteType::F32,
            ConcreteType::Char,
            ConcreteType::I64,
            ConcreteType::I32,
            ConcreteType::I16,
            ConcreteType::I8,
            ConcreteType::U64,
            ConcreteType::U32,
            ConcreteType::U16,
            ConcreteType::U8,
            ConcreteType::Bool,
            ConcreteType::String,
            ConcreteType::Struct(NamedTypeId::placeholder(StructLayoutId(0))),
            ConcreteType::Array(Box::new(ConcreteType::I64)),
            ConcreteType::HashMap(Box::new(ConcreteType::String), Box::new(ConcreteType::I64)),
            ConcreteType::Option(Box::new(ConcreteType::I64)),
            ConcreteType::Result(Box::new(ConcreteType::I64), Box::new(ConcreteType::String)),
            ConcreteType::Enum(NamedTypeId::placeholder(EnumLayoutId(0))),
            ConcreteType::Closure(ClosureTypeId(0)),
            ConcreteType::Function(FunctionTypeId(0)),
            ConcreteType::Pointer(Box::new(ConcreteType::U8)),
            ConcreteType::Tuple(vec![ConcreteType::I64, ConcreteType::String]),
            ConcreteType::Decimal,
            ConcreteType::BigInt,
            ConcreteType::DateTime,
            ConcreteType::HashSet(Box::new(ConcreteType::String)),
            ConcreteType::Deque(Box::new(ConcreteType::I64)),
            ConcreteType::PriorityQueue,
            ConcreteType::Channel(Box::new(ConcreteType::I64)),
            ConcreteType::Mutex(Box::new(ConcreteType::I64)),
            ConcreteType::Atomic,
            ConcreteType::Lazy(Box::new(ConcreteType::I64)),
        ]
    }

    #[test]
    fn every_concrete_capture_type_issues_a_supported_kinded_carrier() {
        let types = representative_concrete_types();
        assert_eq!(types.len(), 33, "update the exhaustive representative set");
        for ty in types {
            let kind = native_kind_from_concrete_type(&ty);
            assert!(
                !matches!(kind, NativeKind::Ptr(heap_kind) if !heap_kind.has_kinded_slot_carrier()),
                "{ty:?} issued carrier-less {kind:?}"
            );
        }
    }

    #[test]
    fn width_specific_native_types_use_scalar_kinds_not_native_scalar() {
        for (ty, expected) in [
            (ConcreteType::I32, NativeKind::Int32),
            (ConcreteType::U32, NativeKind::UInt32),
            (ConcreteType::F32, NativeKind::Float32),
            (ConcreteType::Char, NativeKind::Char),
        ] {
            assert_eq!(native_kind_from_concrete_type(&ty), expected);
        }
    }

    #[test]
    #[should_panic(expected = "Void is not a well-formed capture type")]
    fn void_cannot_issue_a_capture_kind() {
        let _ = native_kind_from_concrete_type(&ConcreteType::Void);
    }
}
