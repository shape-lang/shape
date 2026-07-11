use crate::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA;
use shape_value::heap_value::HeapKind;
use shape_value::{KindedSlot, NativeKind};

/// Exhaustive top-level semantic categories exposed by typed comptime
/// reflection. Payload-bearing compiler descriptors refine these categories;
/// there is deliberately no unknown or inference-variable arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrozenTypeCategory {
    Primitive,
    Never,
    Parameter,
    Nominal,
    Tuple,
    Record,
    Callable,
    Reference,
    Union,
    Erased,
}

/// Return a diagnostic when a compile-stage capability is about to be baked
/// into ordinary runtime code. Scalar comptime results remain liftable; typed
/// reflection capabilities must be consumed before the stage boundary.
pub fn runtime_lift_rejection(value: &KindedSlot) -> Option<&'static str> {
    if value.kind() != NativeKind::Ptr(HeapKind::TypedObject) {
        return None;
    }
    let storage = value.as_typed_object_storage()?;
    let schema = crate::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)?;
    match schema.name.as_str() {
        COMPTIME_FROZEN_TYPE_REF_SCHEMA => {
            Some("TypeRef is a comptime-only compiler capability and cannot enter runtime code")
        }
        "FrozenTypeCategory" => Some(
            "FrozenTypeCategory is comptime-only reflection data and cannot enter runtime code",
        ),
        _ => None,
    }
}

impl FrozenTypeCategory {
    pub const ALL: [Self; 10] = [
        Self::Primitive,
        Self::Never,
        Self::Parameter,
        Self::Nominal,
        Self::Tuple,
        Self::Record,
        Self::Callable,
        Self::Reference,
        Self::Union,
        Self::Erased,
    ];

    pub const fn variant_name(self) -> &'static str {
        match self {
            Self::Primitive => "Primitive",
            Self::Never => "Never",
            Self::Parameter => "Parameter",
            Self::Nominal => "Nominal",
            Self::Tuple => "Tuple",
            Self::Record => "Record",
            Self::Callable => "Callable",
            Self::Reference => "Reference",
            Self::Union => "Union",
            Self::Erased => "Erased",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn frozen_type_category_catalog_is_complete_ordered_and_unique() {
        let names: Vec<_> = FrozenTypeCategory::ALL
            .into_iter()
            .map(FrozenTypeCategory::variant_name)
            .collect();
        assert_eq!(
            names,
            [
                "Primitive",
                "Never",
                "Parameter",
                "Nominal",
                "Tuple",
                "Record",
                "Callable",
                "Reference",
                "Union",
                "Erased",
            ]
        );
        assert_eq!(names.iter().copied().collect::<HashSet<_>>().len(), 10);
        assert!(!names.contains(&"Unknown"));
    }
}
