//! Concrete monomorphized type for v2 runtime.
//!
//! `ConcreteType` replaces `SlotKind` with richer type information that flows
//! from type inference through the bytecode compiler, VM, and JIT. Every local,
//! parameter, field, return value, and collection element has a `ConcreteType`
//! at compile time — no unresolved type variables survive past compilation.
//!
//! This is the foundation for monomorphization: generic functions like
//! `map<T, U>` are specialized per `ConcreteType` instantiation.

use serde::{Deserialize, Serialize};

/// Opaque ID into a registry of struct layouts (resolved at compile time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StructLayoutId(pub u32);

/// Opaque ID into a registry of enum layouts (resolved at compile time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnumLayoutId(pub u32);

/// Opaque ID into a registry of closure capture layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClosureTypeId(pub u32);

/// Opaque ID into a registry of function signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FunctionTypeId(pub u32);

/// Fully resolved, monomorphized type. No type variables, no generics.
///
/// Every expression and local slot in compiled bytecode has exactly one
/// `ConcreteType`. The compiler resolves all `Type::Variable` and
/// `Type::Generic` to `ConcreteType` after type inference.
///
/// The discriminant is stored as `u8` for compact bytecode encoding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConcreteType {
    /// f64 — the default `number` type.
    F64,
    /// i64 — the default `int` type (i48 in NaN-boxed representation).
    I64,
    /// i32
    I32,
    /// i16
    I16,
    /// i8
    I8,
    /// u64
    U64,
    /// u32
    U32,
    /// u16
    U16,
    /// u8
    U8,
    /// bool
    Bool,
    /// Interned string (*const StringObj).
    String,
    /// Typed struct with compile-time field layout.
    Struct(StructLayoutId),
    /// Homogeneous typed array with known element type.
    /// `Array<number>` → `Array(Box::new(ConcreteType::F64))`.
    Array(Box<ConcreteType>),
    /// Typed hash map with known key and value types.
    HashMap(Box<ConcreteType>, Box<ConcreteType>),
    /// Nullable type — `T?` / `Option<T>`.
    Option(Box<ConcreteType>),
    /// Result type — `Result<T, E>`.
    Result(Box<ConcreteType>, Box<ConcreteType>),
    /// Typed enum with compile-time variant layouts.
    Enum(EnumLayoutId),
    /// Closure with typed capture slots.
    Closure(ClosureTypeId),
    /// Function pointer with known signature.
    Function(FunctionTypeId),
    /// Raw typed pointer (for FFI / extern C).
    Pointer(Box<ConcreteType>),
    /// Tuple with known element types.
    Tuple(Vec<ConcreteType>),
    /// Void (unit) — no value.
    Void,
    /// Decimal (rust_decimal::Decimal).
    Decimal,
    /// BigInt (arbitrary precision integer).
    BigInt,
    /// DateTime.
    DateTime,
}

impl ConcreteType {
    /// Size in bytes for stack storage (all values stored as 8-byte slots).
    #[inline]
    pub fn stack_size(&self) -> usize {
        8 // All values occupy one 8-byte stack slot
    }

    /// Natural alignment for this type when stored in a struct.
    #[inline]
    pub fn alignment(&self) -> usize {
        match self {
            ConcreteType::I8 | ConcreteType::U8 | ConcreteType::Bool => 1,
            ConcreteType::I16 | ConcreteType::U16 => 2,
            ConcreteType::I32 | ConcreteType::U32 => 4,
            _ => 8, // f64, i64, u64, pointers, etc.
        }
    }

    /// Size in bytes when stored in a struct field (not on stack).
    #[inline]
    pub fn field_size(&self) -> usize {
        match self {
            ConcreteType::I8 | ConcreteType::U8 | ConcreteType::Bool => 1,
            ConcreteType::I16 | ConcreteType::U16 => 2,
            ConcreteType::I32 | ConcreteType::U32 => 4,
            _ => 8,
        }
    }

    /// Whether this type is a numeric type (integer or float).
    #[inline]
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            ConcreteType::F64
                | ConcreteType::I64
                | ConcreteType::I32
                | ConcreteType::I16
                | ConcreteType::I8
                | ConcreteType::U64
                | ConcreteType::U32
                | ConcreteType::U16
                | ConcreteType::U8
                | ConcreteType::Decimal
                | ConcreteType::BigInt
        )
    }

    /// Whether this type is an integer type.
    #[inline]
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            ConcreteType::I64
                | ConcreteType::I32
                | ConcreteType::I16
                | ConcreteType::I8
                | ConcreteType::U64
                | ConcreteType::U32
                | ConcreteType::U16
                | ConcreteType::U8
        )
    }

    /// Whether this type is a heap-allocated reference type.
    #[inline]
    pub fn is_heap(&self) -> bool {
        matches!(
            self,
            ConcreteType::String
                | ConcreteType::Struct(_)
                | ConcreteType::Array(_)
                | ConcreteType::HashMap(_, _)
                | ConcreteType::Enum(_)
                | ConcreteType::Closure(_)
                | ConcreteType::Pointer(_)
                | ConcreteType::BigInt
                | ConcreteType::Decimal
                | ConcreteType::DateTime
        )
    }

    /// Whether this is a primitive scalar that fits in a register.
    #[inline]
    pub fn is_scalar(&self) -> bool {
        matches!(
            self,
            ConcreteType::F64
                | ConcreteType::I64
                | ConcreteType::I32
                | ConcreteType::I16
                | ConcreteType::I8
                | ConcreteType::U64
                | ConcreteType::U32
                | ConcreteType::U16
                | ConcreteType::U8
                | ConcreteType::Bool
        )
    }

    /// Convert to the corresponding `FieldKind` for struct layout computation.
    pub fn to_field_kind(&self) -> super::struct_layout::FieldKind {
        use super::struct_layout::FieldKind;
        match self {
            ConcreteType::F64 => FieldKind::F64,
            ConcreteType::I64 => FieldKind::I64,
            ConcreteType::I32 => FieldKind::I32,
            ConcreteType::I16 => FieldKind::I16,
            ConcreteType::I8 => FieldKind::I8,
            ConcreteType::U64 => FieldKind::U64,
            ConcreteType::U32 => FieldKind::U32,
            ConcreteType::U16 => FieldKind::U16,
            ConcreteType::U8 => FieldKind::U8,
            ConcreteType::Bool => FieldKind::Bool,
            // All reference/heap types are pointer-sized
            _ => FieldKind::Ptr,
        }
    }

    /// Generate a monomorphization key string for specialization caching.
    /// e.g., `"f64"`, `"array_i64"`, `"hashmap_string_f64"`
    pub fn mono_key(&self) -> String {
        match self {
            ConcreteType::F64 => "f64".into(),
            ConcreteType::I64 => "i64".into(),
            ConcreteType::I32 => "i32".into(),
            ConcreteType::I16 => "i16".into(),
            ConcreteType::I8 => "i8".into(),
            ConcreteType::U64 => "u64".into(),
            ConcreteType::U32 => "u32".into(),
            ConcreteType::U16 => "u16".into(),
            ConcreteType::U8 => "u8".into(),
            ConcreteType::Bool => "bool".into(),
            ConcreteType::String => "string".into(),
            ConcreteType::Struct(id) => format!("struct_{}", id.0),
            ConcreteType::Array(elem) => format!("array_{}", elem.mono_key()),
            ConcreteType::HashMap(k, v) => {
                format!("hashmap_{}_{}", k.mono_key(), v.mono_key())
            }
            ConcreteType::Option(inner) => format!("option_{}", inner.mono_key()),
            ConcreteType::Result(ok, err) => {
                format!("result_{}_{}", ok.mono_key(), err.mono_key())
            }
            ConcreteType::Enum(id) => format!("enum_{}", id.0),
            ConcreteType::Closure(id) => format!("closure_{}", id.0),
            ConcreteType::Function(id) => format!("fn_{}", id.0),
            ConcreteType::Pointer(inner) => format!("ptr_{}", inner.mono_key()),
            ConcreteType::Tuple(elems) => {
                let parts: Vec<_> = elems.iter().map(|e| e.mono_key()).collect();
                format!("tuple_{}", parts.join("_"))
            }
            ConcreteType::Void => "void".into(),
            ConcreteType::Decimal => "decimal".into(),
            ConcreteType::BigInt => "bigint".into(),
            ConcreteType::DateTime => "datetime".into(),
        }
    }

    /// Compact type tag for bytecode encoding (single byte).
    pub fn type_tag(&self) -> u8 {
        match self {
            ConcreteType::F64 => 0,
            ConcreteType::I64 => 1,
            ConcreteType::I32 => 2,
            ConcreteType::I16 => 3,
            ConcreteType::I8 => 4,
            ConcreteType::U64 => 5,
            ConcreteType::U32 => 6,
            ConcreteType::U16 => 7,
            ConcreteType::U8 => 8,
            ConcreteType::Bool => 9,
            ConcreteType::String => 10,
            ConcreteType::Struct(_) => 11,
            ConcreteType::Array(_) => 12,
            ConcreteType::HashMap(_, _) => 13,
            ConcreteType::Option(_) => 14,
            ConcreteType::Result(_, _) => 15,
            ConcreteType::Enum(_) => 16,
            ConcreteType::Closure(_) => 17,
            ConcreteType::Function(_) => 18,
            ConcreteType::Pointer(_) => 19,
            ConcreteType::Tuple(_) => 20,
            ConcreteType::Void => 21,
            ConcreteType::Decimal => 22,
            ConcreteType::BigInt => 23,
            ConcreteType::DateTime => 24,
        }
    }
}

/// Convert from `FieldKind` (struct layout) to `ConcreteType`.
impl From<super::struct_layout::FieldKind> for ConcreteType {
    fn from(fk: super::struct_layout::FieldKind) -> Self {
        use super::struct_layout::FieldKind;
        match fk {
            FieldKind::F64 => ConcreteType::F64,
            FieldKind::I64 => ConcreteType::I64,
            FieldKind::I32 => ConcreteType::I32,
            FieldKind::I16 => ConcreteType::I16,
            FieldKind::I8 => ConcreteType::I8,
            FieldKind::U64 => ConcreteType::U64,
            FieldKind::U32 => ConcreteType::U32,
            FieldKind::U16 => ConcreteType::U16,
            FieldKind::U8 => ConcreteType::U8,
            FieldKind::Bool => ConcreteType::Bool,
            // Ptr is an opaque pointer — caller must know the pointed-to type
            FieldKind::Ptr => ConcreteType::Pointer(Box::new(ConcreteType::Void)),
        }
    }
}

impl std::fmt::Display for ConcreteType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConcreteType::F64 => write!(f, "number"),
            ConcreteType::I64 => write!(f, "int"),
            ConcreteType::I32 => write!(f, "i32"),
            ConcreteType::I16 => write!(f, "i16"),
            ConcreteType::I8 => write!(f, "i8"),
            ConcreteType::U64 => write!(f, "u64"),
            ConcreteType::U32 => write!(f, "u32"),
            ConcreteType::U16 => write!(f, "u16"),
            ConcreteType::U8 => write!(f, "u8"),
            ConcreteType::Bool => write!(f, "bool"),
            ConcreteType::String => write!(f, "string"),
            ConcreteType::Struct(id) => write!(f, "Struct#{}", id.0),
            ConcreteType::Array(elem) => write!(f, "Array<{elem}>"),
            ConcreteType::HashMap(k, v) => write!(f, "HashMap<{k}, {v}>"),
            ConcreteType::Option(inner) => write!(f, "{inner}?"),
            ConcreteType::Result(ok, err) => write!(f, "Result<{ok}, {err}>"),
            ConcreteType::Enum(id) => write!(f, "Enum#{}", id.0),
            ConcreteType::Closure(id) => write!(f, "Closure#{}", id.0),
            ConcreteType::Function(id) => write!(f, "Function#{}", id.0),
            ConcreteType::Pointer(inner) => write!(f, "ptr<{inner}>"),
            ConcreteType::Tuple(elems) => {
                write!(f, "(")?;
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{e}")?;
                }
                write!(f, ")")
            }
            ConcreteType::Void => write!(f, "void"),
            ConcreteType::Decimal => write!(f, "decimal"),
            ConcreteType::BigInt => write!(f, "bigint"),
            ConcreteType::DateTime => write!(f, "DateTime"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mono_key_primitives() {
        assert_eq!(ConcreteType::F64.mono_key(), "f64");
        assert_eq!(ConcreteType::I64.mono_key(), "i64");
        assert_eq!(ConcreteType::Bool.mono_key(), "bool");
        assert_eq!(ConcreteType::String.mono_key(), "string");
    }

    #[test]
    fn test_mono_key_composites() {
        let arr_f64 = ConcreteType::Array(Box::new(ConcreteType::F64));
        assert_eq!(arr_f64.mono_key(), "array_f64");

        let map = ConcreteType::HashMap(
            Box::new(ConcreteType::String),
            Box::new(ConcreteType::I64),
        );
        assert_eq!(map.mono_key(), "hashmap_string_i64");

        let nested = ConcreteType::Array(Box::new(ConcreteType::Array(Box::new(
            ConcreteType::I32,
        ))));
        assert_eq!(nested.mono_key(), "array_array_i32");
    }

    #[test]
    fn test_type_tags_unique() {
        let types = vec![
            ConcreteType::F64,
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
            ConcreteType::Struct(StructLayoutId(0)),
            ConcreteType::Array(Box::new(ConcreteType::F64)),
            ConcreteType::HashMap(Box::new(ConcreteType::String), Box::new(ConcreteType::F64)),
            ConcreteType::Option(Box::new(ConcreteType::I64)),
            ConcreteType::Result(Box::new(ConcreteType::I64), Box::new(ConcreteType::String)),
            ConcreteType::Enum(EnumLayoutId(0)),
            ConcreteType::Closure(ClosureTypeId(0)),
            ConcreteType::Function(FunctionTypeId(0)),
            ConcreteType::Pointer(Box::new(ConcreteType::U8)),
            ConcreteType::Tuple(vec![ConcreteType::I64, ConcreteType::F64]),
            ConcreteType::Void,
            ConcreteType::Decimal,
            ConcreteType::BigInt,
            ConcreteType::DateTime,
        ];
        let tags: Vec<u8> = types.iter().map(|t| t.type_tag()).collect();
        let unique: std::collections::HashSet<u8> = tags.iter().copied().collect();
        assert_eq!(tags.len(), unique.len(), "type tags must be unique");
    }

    #[test]
    fn test_field_kind_roundtrip() {
        use super::super::struct_layout::FieldKind;
        let kinds = [
            FieldKind::F64,
            FieldKind::I64,
            FieldKind::I32,
            FieldKind::I16,
            FieldKind::I8,
            FieldKind::U64,
            FieldKind::U32,
            FieldKind::U16,
            FieldKind::U8,
            FieldKind::Bool,
        ];
        for kind in kinds {
            let ct = ConcreteType::from(kind);
            let back = ct.to_field_kind();
            assert_eq!(kind, back);
        }
    }

    #[test]
    fn test_is_numeric() {
        assert!(ConcreteType::F64.is_numeric());
        assert!(ConcreteType::I64.is_numeric());
        assert!(ConcreteType::U8.is_numeric());
        assert!(ConcreteType::Decimal.is_numeric());
        assert!(!ConcreteType::Bool.is_numeric());
        assert!(!ConcreteType::String.is_numeric());
    }

    #[test]
    fn test_is_heap() {
        assert!(ConcreteType::String.is_heap());
        assert!(ConcreteType::Array(Box::new(ConcreteType::F64)).is_heap());
        assert!(!ConcreteType::F64.is_heap());
        assert!(!ConcreteType::Bool.is_heap());
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", ConcreteType::F64), "number");
        assert_eq!(format!("{}", ConcreteType::I64), "int");
        assert_eq!(
            format!("{}", ConcreteType::Array(Box::new(ConcreteType::F64))),
            "Array<number>"
        );
        assert_eq!(
            format!("{}", ConcreteType::Option(Box::new(ConcreteType::I64))),
            "int?"
        );
    }
}
