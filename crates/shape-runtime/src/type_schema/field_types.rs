//! Field type definitions for type schemas
//!
//! This module defines the types of fields that can be part of a schema,
//! including primitives (F64, I64, Bool), composite types (String, Array),
//! and dynamic types (Any).

use shape_value::{HeapKind, NativeKind};

/// Error returned when a `FieldType` cannot be projected to a strict-typed
/// `NativeKind`. The current source of error is `FieldType::Any`, which
/// the strict-typing plan forbids; legacy schemas may still carry it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKindError {
    /// `FieldType::Any` has no strict-typed `NativeKind` projection. Per
    /// the strict-typing plan (`docs/defections.md` 2026-05-06 Phase 2b
    /// watchlist), parametric/generic NativeKind variants are not the
    /// answer — `Any`-typed fields must be eliminated from schemas.
    AnyTypeNotStrictlyTyped,
}

impl std::fmt::Display for FieldKindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldKindError::AnyTypeNotStrictlyTyped => {
                write!(f, "FieldType::Any has no strict-typed NativeKind projection")
            }
        }
    }
}

impl std::error::Error for FieldKindError {}

/// Type of a field in a schema
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FieldType {
    /// 64-bit floating point number
    F64,
    /// 64-bit signed integer
    I64,
    /// Boolean (stored as u64 for alignment)
    Bool,
    /// String (pointer to heap-allocated String)
    String,
    /// Timestamp (i64 milliseconds since epoch)
    Timestamp,
    /// Array (pointer to heap-allocated Vec)
    Array(Box<FieldType>),
    /// Nested object of known type
    Object(String),
    /// Decimal (stored as f64 for TypedObject, reconstructed on read)
    Decimal,
    /// Any/dynamic type (uses HashMap access)
    Any,
    /// Width-specific integer types (stored as i64 in NaN-boxed slot, truncated on write)
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    U64,
}

impl std::fmt::Display for FieldType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldType::F64 => write!(f, "number"),
            FieldType::I64 => write!(f, "int"),
            FieldType::Bool => write!(f, "bool"),
            FieldType::String => write!(f, "string"),
            FieldType::Timestamp => write!(f, "timestamp"),
            FieldType::Decimal => write!(f, "decimal"),
            FieldType::Any => write!(f, "any"),
            FieldType::Array(inner) => write!(f, "{}[]", inner),
            FieldType::Object(name) => write!(f, "{}", name),
            FieldType::I8 => write!(f, "i8"),
            FieldType::U8 => write!(f, "u8"),
            FieldType::I16 => write!(f, "i16"),
            FieldType::U16 => write!(f, "u16"),
            FieldType::I32 => write!(f, "i32"),
            FieldType::U32 => write!(f, "u32"),
            FieldType::U64 => write!(f, "u64"),
        }
    }
}

impl FieldType {
    /// Check if a value of type `other` can be assigned to a field of type `self`.
    ///
    /// Rules:
    /// - Same type → compatible
    /// - Any on either side → compatible
    /// - I64 → F64 → compatible (int widens to number losslessly)
    /// - I64 → Decimal → compatible (int widens to decimal losslessly)
    /// - F64 → I64 → NOT compatible (narrowing)
    /// - Decimal → I64 → NOT compatible (narrowing)
    /// - Decimal → F64 → NOT compatible (precision loss)
    /// - F64 → Decimal → NOT compatible (conceptually different types)
    pub fn is_compatible_with(&self, value_type: &FieldType) -> bool {
        if self == value_type {
            return true;
        }
        if matches!(self, FieldType::Any) || matches!(value_type, FieldType::Any) {
            return true;
        }
        // Allow implicit widening: int → number, int → decimal, width int → I64/F64
        match (self, value_type) {
            (FieldType::F64, FieldType::I64) => true,
            (FieldType::Decimal, FieldType::I64) => true,
            // Width int types can widen to I64 or F64
            (FieldType::I64, vt) if vt.is_width_integer() => true,
            (FieldType::F64, vt) if vt.is_width_integer() => true,
            _ => false,
        }
    }

    /// Size of this field type in bytes
    pub fn size(&self) -> usize {
        match self {
            FieldType::F64 => 8,
            FieldType::I64 => 8,
            FieldType::Bool => 8,   // Padded for alignment
            FieldType::String => 8, // Pointer
            FieldType::Timestamp => 8,
            FieldType::Array(_) => 8,  // Pointer
            FieldType::Object(_) => 8, // Pointer
            FieldType::Decimal => 8,   // Stored as f64
            FieldType::Any => 8,       // NaN-boxed value
            FieldType::I8
            | FieldType::U8
            | FieldType::I16
            | FieldType::U16
            | FieldType::I32
            | FieldType::U32
            | FieldType::U64 => 8, // NaN-boxed slot
        }
    }

    /// Alignment requirement for this field type
    pub fn alignment(&self) -> usize {
        8 // All fields are 8-byte aligned for simplicity
    }

    /// Returns true if this field type could potentially hold a callable value
    /// (closure, function reference). Primitive numeric/bool/string types are
    /// never callable. `Any`, `Object`, and `Array` might hold callables.
    pub fn is_potentially_callable(&self) -> bool {
        matches!(
            self,
            FieldType::Any | FieldType::Object(_) | FieldType::Array(_)
        )
    }

    /// Project this field type to its strict-typed marshal/wire/snapshot
    /// `NativeKind` discriminator. Used by the wire/snapshot kind-threading
    /// path (Phase 2b) and by the marshal layer when a TypedObject's per-
    /// slot kind is needed.
    ///
    /// `FieldType::Any` returns
    /// [`FieldKindError::AnyTypeNotStrictlyTyped`] — callers must handle
    /// that case explicitly. The strict-typing plan forbids `Any`-typed
    /// fields in new code; legacy schemas with `Any` fields are the
    /// only consumers of the error variant.
    ///
    /// `Decimal` is stored as `f64` in `TypedObject` slots (lossy) per
    /// the existing layout — kind is `Float64` accordingly.
    pub fn to_native_kind(&self) -> Result<NativeKind, FieldKindError> {
        match self {
            Self::F64 => Ok(NativeKind::Float64),
            Self::I64 => Ok(NativeKind::Int64),
            Self::Bool => Ok(NativeKind::Bool),
            Self::String => Ok(NativeKind::String),
            Self::Timestamp => Ok(NativeKind::Int64),
            Self::Decimal => Ok(NativeKind::Float64),
            Self::Array(_) => Ok(NativeKind::Ptr(HeapKind::TypedArray)),
            Self::Object(_) => Ok(NativeKind::Ptr(HeapKind::TypedObject)),
            Self::I8 => Ok(NativeKind::Int8),
            Self::U8 => Ok(NativeKind::UInt8),
            Self::I16 => Ok(NativeKind::Int16),
            Self::U16 => Ok(NativeKind::UInt16),
            Self::I32 => Ok(NativeKind::Int32),
            Self::U32 => Ok(NativeKind::UInt32),
            Self::U64 => Ok(NativeKind::UInt64),
            Self::Any => Err(FieldKindError::AnyTypeNotStrictlyTyped),
        }
    }

    /// Returns true if this is a sub-64 or unsigned-64 integer width type.
    pub fn is_width_integer(&self) -> bool {
        matches!(
            self,
            FieldType::I8
                | FieldType::U8
                | FieldType::I16
                | FieldType::U16
                | FieldType::I32
                | FieldType::U32
                | FieldType::U64
        )
    }
}

/// A single annotation on a field (e.g. `@alias("Close Price")`)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldAnnotation {
    pub name: String,
    pub args: Vec<String>,
}

/// Definition of a single field in a type schema
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldDef {
    /// Field name
    pub name: String,
    /// Field type
    pub field_type: FieldType,
    /// Byte offset from start of object data
    pub offset: usize,
    /// Field index (for fast lookup)
    pub index: u16,
    /// All annotations on this field
    pub annotations: Vec<FieldAnnotation>,
}

impl FieldDef {
    /// Create a new field definition
    pub fn new(name: impl Into<String>, field_type: FieldType, offset: usize, index: u16) -> Self {
        Self {
            name: name.into(),
            field_type,
            offset,
            index,
            annotations: vec![],
        }
    }

    /// Returns the wire name for this field: the `@alias("...")` value if present,
    /// otherwise the field name. Used by Arrow binding, FFI marshalling, and
    /// any other serialization boundary.
    pub fn wire_name(&self) -> &str {
        for ann in &self.annotations {
            if ann.name == "alias" && !ann.args.is_empty() {
                return &ann.args[0];
            }
        }
        &self.name
    }
}

/// Convert SemanticType to FieldType for JIT schema creation
pub(crate) fn semantic_to_field_type(
    semantic: &crate::type_system::SemanticType,
    is_optional: bool,
) -> FieldType {
    use crate::type_system::SemanticType;

    // If the field is optional, we use Any to handle the NaN sentinel
    if is_optional {
        return FieldType::Any;
    }

    match semantic {
        SemanticType::Number => FieldType::F64,
        SemanticType::Integer => FieldType::I64,
        SemanticType::Bool => FieldType::Bool,
        SemanticType::String => FieldType::String,
        SemanticType::Array(elem) => {
            FieldType::Array(Box::new(semantic_to_field_type(elem, false)))
        }
        SemanticType::Option(_) => FieldType::Any, // Optional values use NaN boxing
        SemanticType::Struct { name, .. } => FieldType::Object(name.clone()),
        SemanticType::Named(name) if name == "Decimal" => FieldType::Decimal,
        SemanticType::Named(name) => FieldType::Object(name.clone()),
        _ => FieldType::Any, // Default to Any for complex types
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_type_sizes() {
        assert_eq!(FieldType::F64.size(), 8);
        assert_eq!(FieldType::I64.size(), 8);
        assert_eq!(FieldType::Bool.size(), 8);
        assert_eq!(FieldType::String.size(), 8);
        assert_eq!(FieldType::Timestamp.size(), 8);
        assert_eq!(FieldType::Array(Box::new(FieldType::F64)).size(), 8);
        assert_eq!(FieldType::Object("Candle".to_string()).size(), 8);
        assert_eq!(FieldType::Any.size(), 8);
    }

    #[test]
    fn test_field_type_alignment() {
        assert_eq!(FieldType::F64.alignment(), 8);
        assert_eq!(FieldType::I64.alignment(), 8);
        assert_eq!(FieldType::Bool.alignment(), 8);
    }

    #[test]
    fn test_field_def_creation() {
        let field = FieldDef::new("test", FieldType::F64, 16, 2);
        assert_eq!(field.name, "test");
        assert_eq!(field.field_type, FieldType::F64);
        assert_eq!(field.offset, 16);
        assert_eq!(field.index, 2);
    }
}
