//! Typed schema carriers for ADR-009 generated captures.

use super::{EnumVariantInfo, TypeSchemaBuilder, TypeSchemaRegistry};

/// Unspellable value-carrier schema for one signature-indexed typed closure
/// capture (`CaptureDescriptor<Sig,I,T,Mode>`). It contains exact identities,
/// the structural position, and the typed mode—never names or rendered types.
pub const COMPTIME_CAPTURE_DESCRIPTOR_SCHEMA: &str = "\u{1}comptime:CaptureDescriptor";

pub(super) fn register(registry: &mut TypeSchemaRegistry) {
    registry.register_enum_scoped(
        crate::comptime_reflection::CAPTURE_MODE_SCHEMA_NAME,
        shape_ast::ast::CaptureMode::ALL
            .into_iter()
            .enumerate()
            .map(|(id, mode)| EnumVariantInfo::new(mode.variant_name(), id as u16, 0))
            .collect(),
    );
    TypeSchemaBuilder::new(COMPTIME_CAPTURE_DESCRIPTOR_SCHEMA)
        .int_field("signature_identity_high")
        .int_field("signature_identity_low")
        .int_field("index")
        .int_field("type_identity_high")
        .int_field("type_identity_low")
        .object_field("mode", crate::comptime_reflection::CAPTURE_MODE_SCHEMA_NAME)
        .register(registry);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_and_closed_mode_axis_register_together() {
        let mut registry = TypeSchemaRegistry::new();
        super::super::register_builtin_schemas(&mut registry);

        let descriptor = registry
            .get(COMPTIME_CAPTURE_DESCRIPTOR_SCHEMA)
            .expect("capture descriptor schema");
        assert_eq!(descriptor.field_count(), 6);
        for field in [
            "signature_identity_high",
            "signature_identity_low",
            "index",
            "type_identity_high",
            "type_identity_low",
            "mode",
        ] {
            assert!(descriptor.get_field(field).is_some(), "{field}");
        }

        let modes = registry
            .get(crate::comptime_reflection::CAPTURE_MODE_SCHEMA_NAME)
            .expect("capture mode schema");
        for variant in ["Move", "Share", "SharedBorrow", "ExclusiveBorrow"] {
            assert!(modes.variant_id(variant).is_some(), "{variant}");
        }
    }
}
