//! Catalog-keyed enum projection for reflection and tooling.

use super::*;

/// Return variants only for sealed reflection enums. Every list is projected
/// from the compiler-owned catalog used by reflection itself; payload structs
/// and user-defined enums deliberately return `None`.
pub fn reflection_enum_variant_names(enum_name: &str) -> Option<Vec<&'static str>> {
    if enum_name == "FrozenTypeCategory" {
        return Some(
            FrozenTypeCategory::ALL
                .into_iter()
                .map(FrozenTypeCategory::variant_name)
                .collect(),
        );
    }
    if enum_name == FROZEN_TYPE_PAYLOAD_ENUM_NAME {
        return Some(
            FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES
                .into_iter()
                .map(FrozenTypeCategory::variant_name)
                .collect(),
        );
    }
    if Some(enum_name) == frozen_type_enabled_payload_type_name(FrozenTypeCategory::Primitive) {
        return Some(FROZEN_PRIMITIVE_VARIANTS.iter().map(|v| v.name).collect());
    }
    if enum_name == INTEGER_WIDTH_SCHEMA_NAME {
        return Some(
            IntegerWidth::ALL
                .into_iter()
                .map(IntegerWidth::variant_name)
                .collect(),
        );
    }
    if enum_name == FLOAT_WIDTH_SCHEMA_NAME {
        return Some(
            FloatWidth::ALL
                .into_iter()
                .map(FloatWidth::variant_name)
                .collect(),
        );
    }
    if enum_name == PARAM_KIND_SCHEMA_NAME {
        return Some(
            ParamKind::ALL
                .into_iter()
                .map(ParamKind::variant_name)
                .collect(),
        );
    }
    if enum_name == PASSING_MODE_SCHEMA_NAME {
        return Some(
            PassingMode::ALL
                .into_iter()
                .map(PassingMode::variant_name)
                .collect(),
        );
    }
    if enum_name == CAPTURE_MODE_SCHEMA_NAME {
        return Some(
            shape_ast::ast::CaptureMode::ALL
                .into_iter()
                .map(shape_ast::ast::CaptureMode::variant_name)
                .collect(),
        );
    }
    if enum_name == NOMINAL_SHAPE_SCHEMA_NAME {
        return Some(
            NominalShape::ALL
                .into_iter()
                .map(NominalShape::variant_name)
                .collect(),
        );
    }
    if enum_name == FIELD_INITIALIZATION_SCHEMA_NAME {
        return Some(
            FieldInitialization::ALL
                .into_iter()
                .map(FieldInitialization::variant_name)
                .collect(),
        );
    }
    None
}
