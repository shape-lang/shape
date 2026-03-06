//! Storage Types
//!
//! Defines how types are physically represented in memory for JIT optimization.
//!
//! Key design decisions:
//! - `Option<f64>` → NaN sentinel (IEEE 754 propagation, zero overhead)
//! - `Option<i64>` → Validity bitmap (no safe sentinel)
//! - `Option<bool>` → u8 with 0/1/2 encoding (SIMD-friendly)
//! - `Result<T>` → Tagged union (Ok is lightweight, Err is heavy)

use super::semantic::SemanticType;
use std::fmt;

/// Storage types - how data is physically represented in memory
#[derive(Clone, Debug, PartialEq)]
pub enum StorageType {
    // === Scalars (direct values) ===
    /// f64 value (NaN = None when used with Option)
    Float64,

    /// i64 value
    Int64,

    /// Boolean value
    Bool,

    /// String value (Arc<str> internally)
    String,

    // === Nullable Scalars (for JIT optimization) ===
    /// f64 with NaN sentinel for None
    /// - Storage: Just f64
    /// - None check: `value.is_nan()`
    /// - Arithmetic: NaN propagates automatically (IEEE 754)
    NullableFloat64,

    /// i64 with separate validity tracking
    /// - Storage: i64 + 1 bit validity
    /// - None check: Check validity bit
    /// - Note: i64::MIN is a valid number, so no sentinel possible
    NullableInt64,

    /// Bool with 3-state encoding
    /// - Storage: u8
    /// - 0 = false, 1 = true, 2 = None
    /// - SIMD-friendly byte comparison
    NullableBool,

    // === Containers ===
    /// Array with homogeneous element type
    Array(Box<StorageType>),

    /// Table (time-indexed columnar data)
    Table {
        element: Box<StorageType>,
        /// True if column can have nulls (uses validity bitmap for non-NaN types)
        has_validity_bitmap: bool,
    },

    /// Object (key-value map)
    Object,

    // === Result Type ===
    /// Result<T, E> as tagged union
    /// - Ok(T): Just the value with a discriminant
    /// - Err(E): Error payload (message, code, details)
    Result {
        ok_storage: Box<StorageType>,
        /// True if using universal error (no custom error type)
        universal_error: bool,
    },

    // === Complex Types ===
    /// Discriminated union with explicit tag
    /// - Used for Option<non-numeric> and enums
    TaggedUnion {
        variants: Vec<(String, Option<StorageType>)>,
    },

    /// Function pointer / closure
    Function,

    /// Reference to user-defined struct
    Struct(String),

    /// Dynamically typed value (escape hatch)
    Dynamic,
}

impl StorageType {
    /// Convert semantic type to storage type
    ///
    /// This is the critical mapping that preserves JIT performance:
    /// - `Option<f64>` becomes `NullableFloat64` (NaN sentinel)
    /// - `Option<i64>` becomes `NullableInt64` (validity bit)
    /// - `Option<bool>` becomes `NullableBool` (u8 encoding)
    pub fn from_semantic(ty: &SemanticType) -> Self {
        if ty.is_number_family() {
            return StorageType::Float64;
        }
        if ty.is_integer_family() {
            return StorageType::Int64;
        }

        match ty {
            // Primitives
            SemanticType::Number => StorageType::Float64,
            SemanticType::Integer => StorageType::Int64,
            SemanticType::Bool => StorageType::Bool,
            SemanticType::String => StorageType::String,

            // Option<T> - use optimal storage based on T
            SemanticType::Option(inner) => match inner.as_ref() {
                inner if inner.is_number_family() => StorageType::NullableFloat64,
                inner if inner.is_integer_family() => StorageType::NullableInt64,
                SemanticType::Bool => StorageType::NullableBool,
                other => StorageType::TaggedUnion {
                    variants: vec![
                        ("None".to_string(), None),
                        ("Some".to_string(), Some(Self::from_semantic(other))),
                    ],
                },
            },

            // Result<T> or Result<T, E>
            SemanticType::Result { ok_type, err_type } => StorageType::Result {
                ok_storage: Box::new(Self::from_semantic(ok_type)),
                universal_error: err_type.is_none(),
            },

            // Vec<T>
            SemanticType::Array(elem) => StorageType::Array(Box::new(Self::from_semantic(elem))),

            // Struct
            SemanticType::Struct { name, .. } => StorageType::Struct(name.clone()),

            // Enum - tagged union
            SemanticType::Enum { variants, .. } => StorageType::TaggedUnion {
                variants: variants
                    .iter()
                    .map(|v| {
                        let payload = v.payload.as_ref().map(Self::from_semantic);
                        (v.name.clone(), payload)
                    })
                    .collect(),
            },

            // Function
            SemanticType::Function(_) => StorageType::Function,

            // Type variables and unresolved - use dynamic
            SemanticType::TypeVar(_) | SemanticType::Generic { .. } | SemanticType::Any => {
                StorageType::Dynamic
            }

            // Named types — resolve known primitives, default to Struct for user types
            SemanticType::Named(name) => {
                match name.as_str() {
                    "Decimal" => StorageType::Float64, // Decimal stored as f64
                    "DateTime" | "Instant" => StorageType::Int64, // Timestamps
                    _ => StorageType::Struct(name.clone()),
                }
            }

            // Special — truly unknown types
            SemanticType::Never | SemanticType::Void => StorageType::Dynamic,
            SemanticType::Interface { .. } => StorageType::Dynamic,
        }
    }

    /// Check if this storage type uses NaN sentinel for nullability
    pub fn uses_nan_sentinel(&self) -> bool {
        matches!(self, StorageType::NullableFloat64)
    }

    /// Check if this storage type needs a validity bitmap
    pub fn needs_validity_bitmap(&self) -> bool {
        matches!(self, StorageType::NullableInt64 | StorageType::NullableBool)
    }

    /// Get the non-nullable version of this type
    pub fn to_non_nullable(&self) -> Self {
        match self {
            StorageType::NullableFloat64 => StorageType::Float64,
            StorageType::NullableInt64 => StorageType::Int64,
            StorageType::NullableBool => StorageType::Bool,
            other => other.clone(),
        }
    }

    /// Get the size in bytes for scalar types (for JIT memory layout)
    pub fn scalar_size(&self) -> Option<usize> {
        match self {
            StorageType::Float64 | StorageType::NullableFloat64 => Some(8),
            StorageType::Int64 | StorageType::NullableInt64 => Some(8),
            StorageType::Bool | StorageType::NullableBool => Some(1),
            _ => None, // Non-scalar types
        }
    }

    /// Check if JIT can optimize this type with SIMD
    pub fn is_simd_compatible(&self) -> bool {
        matches!(
            self,
            StorageType::Float64
                | StorageType::NullableFloat64
                | StorageType::Int64
                | StorageType::NullableInt64
        )
    }
}

impl fmt::Display for StorageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageType::Float64 => write!(f, "f64"),
            StorageType::Int64 => write!(f, "i64"),
            StorageType::Bool => write!(f, "bool"),
            StorageType::String => write!(f, "string"),
            StorageType::NullableFloat64 => write!(f, "f64?"),
            StorageType::NullableInt64 => write!(f, "i64?"),
            StorageType::NullableBool => write!(f, "bool?"),
            StorageType::Array(elem) => write!(f, "[{}]", elem),
            StorageType::Table { element, .. } => write!(f, "Table<{}>", element),
            StorageType::Object => write!(f, "object"),
            StorageType::Result { ok_storage, .. } => write!(f, "Result<{}>", ok_storage),
            StorageType::TaggedUnion { variants } => {
                write!(f, "union{{")?;
                for (i, (name, _)) in variants.iter().enumerate() {
                    if i > 0 {
                        write!(f, "|")?;
                    }
                    write!(f, "{}", name)?;
                }
                write!(f, "}}")
            }
            StorageType::Function => write!(f, "fn"),
            StorageType::Struct(name) => write!(f, "{}", name),
            StorageType::Dynamic => write!(f, "dynamic"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_option_f64_uses_nan() {
        let ty = SemanticType::option(SemanticType::Number);
        let storage = StorageType::from_semantic(&ty);
        assert_eq!(storage, StorageType::NullableFloat64);
        assert!(storage.uses_nan_sentinel());
    }

    #[test]
    fn test_option_i64_uses_bitmap() {
        let ty = SemanticType::option(SemanticType::Integer);
        let storage = StorageType::from_semantic(&ty);
        assert_eq!(storage, StorageType::NullableInt64);
        assert!(storage.needs_validity_bitmap());
    }

    #[test]
    fn test_option_bool_uses_u8() {
        let ty = SemanticType::option(SemanticType::Bool);
        let storage = StorageType::from_semantic(&ty);
        assert_eq!(storage, StorageType::NullableBool);
    }

    #[test]
    fn test_option_width_aware_numeric_types_use_numeric_storage() {
        let ty_i16 = SemanticType::option(SemanticType::Named("i16".to_string()));
        let storage_i16 = StorageType::from_semantic(&ty_i16);
        assert_eq!(storage_i16, StorageType::NullableInt64);

        let ty_f32 = SemanticType::option(SemanticType::Named("f32".to_string()));
        let storage_f32 = StorageType::from_semantic(&ty_f32);
        assert_eq!(storage_f32, StorageType::NullableFloat64);
    }

    #[test]
    fn test_table_storage_display() {
        let storage = StorageType::Table {
            element: Box::new(StorageType::Float64),
            has_validity_bitmap: false,
        };
        assert_eq!(format!("{}", storage), "Table<f64>");
    }

    #[test]
    fn test_result_storage() {
        let ty = SemanticType::result(SemanticType::Number);
        let storage = StorageType::from_semantic(&ty);
        match storage {
            StorageType::Result {
                universal_error, ..
            } => {
                assert!(universal_error);
            }
            _ => panic!("Expected Result storage"),
        }
    }

    #[test]
    fn test_scalar_sizes() {
        assert_eq!(StorageType::Float64.scalar_size(), Some(8));
        assert_eq!(StorageType::Int64.scalar_size(), Some(8));
        assert_eq!(StorageType::Bool.scalar_size(), Some(1));
        assert_eq!(StorageType::String.scalar_size(), None);
    }

    #[test]
    fn test_named_type_resolution() {
        assert_eq!(
            StorageType::from_semantic(&SemanticType::Named("Candle".to_string())),
            StorageType::Struct("Candle".to_string())
        );
        assert_eq!(
            StorageType::from_semantic(&SemanticType::Named("Decimal".to_string())),
            StorageType::Float64
        );
        assert_eq!(
            StorageType::from_semantic(&SemanticType::Named("DateTime".to_string())),
            StorageType::Int64
        );
        assert_eq!(
            StorageType::from_semantic(&SemanticType::Named("Instant".to_string())),
            StorageType::Int64
        );
        // Verify Never/Void still resolve to Dynamic
        assert_eq!(
            StorageType::from_semantic(&SemanticType::Never),
            StorageType::Dynamic
        );
        assert_eq!(
            StorageType::from_semantic(&SemanticType::Void),
            StorageType::Dynamic
        );
    }
}
