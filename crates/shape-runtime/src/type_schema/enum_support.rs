//! Enum type support for type schemas
//!
//! This module provides support for sum types (enums) with variant information,
//! allowing TypedObject optimization for enum types like Option<T> and Result<T, E>.

use std::collections::HashMap;

/// Payload shape carried by a single enum variant.
///
/// Drives `print()` / `Display` rendering at the W18.0 enum-variant arm
/// (`crates/shape-vm/src/executor/printing.rs::format_enum_typed_object`)
/// so unit / tuple / struct variants render `Red` / `Blue(42)` /
/// `Point { x: 1, y: 2 }` respectively per User 2026-05-23 Item 1.
///
/// The runtime layout in TypedObjectStorage is unchanged — payload fields
/// are still `__payload_0` / `__payload_1` / ... at offsets 8/16/...; the
/// kind here only describes the surface-syntax shape that produced the
/// variant so print() can recover the user-visible name/form.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub enum EnumVariantKind {
    /// Unit variant — no payload. `Red`.
    #[default]
    Unit,
    /// Tuple variant — N positional payloads. `Blue(42)`, `Pair(1, 2)`.
    Tuple,
    /// Struct variant — N named-field payloads. `Point { x: 1, y: 2 }`.
    /// `field_names[i]` is the source-level name of the i-th `__payload_i`.
    Struct(Vec<String>),
}

/// Information about an enum variant for TypedObject optimization
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EnumVariantInfo {
    /// Variant name (e.g., "Some", "None", "Ok", "Err")
    pub name: String,
    /// Unique variant ID (0, 1, 2...)
    pub id: u16,
    /// Number of payload fields for this variant
    pub payload_fields: u16,
    /// Surface-syntax kind that produced this variant — drives W18.0
    /// `print()` rendering shape (Unit / Tuple / Struct{field_names}).
    /// Defaults to `Unit` when `payload_fields == 0`, `Tuple` otherwise,
    /// for back-compat with `EnumVariantInfo::new(name, id, count)` call
    /// sites that don't carry struct-field metadata (e.g. the stdlib
    /// Option/Result aliases registered in `registry.rs`).
    #[serde(default)]
    pub kind: EnumVariantKind,
}

impl EnumVariantInfo {
    /// Create a new enum variant info. Infers `kind` from `payload_fields`:
    /// 0 → `Unit`, >0 → `Tuple`. Use [`Self::new_struct`] for struct variants.
    pub fn new(name: impl Into<String>, id: u16, payload_fields: u16) -> Self {
        let kind = if payload_fields == 0 {
            EnumVariantKind::Unit
        } else {
            EnumVariantKind::Tuple
        };
        Self {
            name: name.into(),
            id,
            payload_fields,
            kind,
        }
    }

    /// Create a struct-shaped variant info. `field_names.len()` is the
    /// payload count.
    pub fn new_struct(name: impl Into<String>, id: u16, field_names: Vec<String>) -> Self {
        let payload_fields = field_names.len() as u16;
        Self {
            name: name.into(),
            id,
            payload_fields,
            kind: EnumVariantKind::Struct(field_names),
        }
    }
}

/// Enum-specific information for TypedObject optimization
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EnumInfo {
    /// All variants of this enum
    pub variants: Vec<EnumVariantInfo>,
    /// Variant lookup by name
    variant_map: HashMap<String, u16>,
}

impl EnumInfo {
    /// Create new enum info with variants
    pub fn new(variants: Vec<EnumVariantInfo>) -> Self {
        let variant_map = variants.iter().map(|v| (v.name.clone(), v.id)).collect();
        Self {
            variants,
            variant_map,
        }
    }

    /// Get variant ID by name
    pub fn variant_id(&self, name: &str) -> Option<u16> {
        self.variant_map.get(name).copied()
    }

    /// Get variant info by ID
    pub fn variant_by_id(&self, id: u16) -> Option<&EnumVariantInfo> {
        self.variants.iter().find(|v| v.id == id)
    }

    /// Get variant info by name
    pub fn variant_by_name(&self, name: &str) -> Option<&EnumVariantInfo> {
        self.variant_id(name).and_then(|id| self.variant_by_id(id))
    }

    /// Maximum payload fields across all variants (for union-style layout)
    pub fn max_payload_fields(&self) -> u16 {
        self.variants
            .iter()
            .map(|v| v.payload_fields)
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enum_variant_info() {
        let variant = EnumVariantInfo::new("Some", 0, 1);
        assert_eq!(variant.name, "Some");
        assert_eq!(variant.id, 0);
        assert_eq!(variant.payload_fields, 1);
    }

    #[test]
    fn test_enum_info() {
        let enum_info = EnumInfo::new(vec![
            EnumVariantInfo::new("Some", 0, 1),
            EnumVariantInfo::new("None", 1, 0),
        ]);

        assert_eq!(enum_info.variants.len(), 2);
        assert_eq!(enum_info.variant_id("Some"), Some(0));
        assert_eq!(enum_info.variant_id("None"), Some(1));
        assert_eq!(enum_info.variant_id("Unknown"), None);
        assert_eq!(enum_info.max_payload_fields(), 1);
    }

    #[test]
    fn test_enum_variant_lookup() {
        let enum_info = EnumInfo::new(vec![
            EnumVariantInfo::new("Ok", 0, 1),
            EnumVariantInfo::new("Err", 1, 1),
        ]);

        let ok_variant = enum_info.variant_by_name("Ok").unwrap();
        assert_eq!(ok_variant.id, 0);
        assert_eq!(ok_variant.payload_fields, 1);

        let err_variant = enum_info.variant_by_id(1).unwrap();
        assert_eq!(err_variant.name, "Err");
    }
}
