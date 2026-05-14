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
    /// f32 — 4-byte single-precision float. ADR-006 §2.7.5 amendment
    /// (Round 19 S1.5, 2026-05-14): scalar concrete type introduced
    /// alongside `NativeKind::Float32` for `Array<f32>` v2-raw
    /// producer paths.
    F32,
    /// `char` — 4-byte Unicode scalar (UTF-32 subset of `u32`). ADR-006
    /// §2.7.5 amendment (Round 19 S1.5, 2026-05-14): scalar concrete
    /// type introduced alongside `NativeKind::Char` for `Array<char>`
    /// v2-raw producer paths.
    Char,
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
    // ── Phase 3 cluster-0 Round 11-trinity 11E (2026-05-13) ─────────────
    //
    // Collection-container and concurrency-primitive arms surfaced by
    // Round 10's W12-jit-call-method-shell-rebuild close: the JIT-side
    // §2.7.5 producing-site conduit cannot stamp parametric return kinds
    // (`HashMap.get → Option<V>`, `Mutex.get → T`, `Lazy.force → T`) and
    // the JIT-side `EnumStore` collection-ctor consumer (Round 10) needed
    // a §2.7.5 in-band classifier rather than an out-of-band MIR-shape
    // override at `mir_compiler/types.rs:467` — the audit's
    // "W17-collection-concrete-types tracked follow-up" closing here.
    //
    // **Single-discriminator discipline (ADR-005 §1).** None of these
    // arms project 1:1 to `HeapKind`. Each parametric arm wraps a
    // `Box<ConcreteType>` for the inner kind (mirror of the existing
    // `Array(Box<ConcreteType>)` / `HashMap(Box<_>, Box<_>)` shape).
    // Nullary arms (`PriorityQueue`, `Atomic`) match a §2.7.18 / §2.7.25
    // landing-time storage decision (i64-only payload) — the ConcreteType
    // is genuinely nullary at landing, not "stripped to be a discriminator".
    // When the typed-payload follow-up amendments land (Phase 2c per
    // §2.7.18 / §2.7.25 "Out-of-scope" notes) these arms grow a
    // `Box<ConcreteType>` parameter at that point.
    //
    /// HashSet with known element type. String-only at landing per
    /// ADR-006 §2.7.15 (`HeapKind::HashSet`); the inner `ConcreteType`
    /// is `String` at construction sites today, and the parametric arm
    /// shape preserves room for the future typed-payload extension.
    HashSet(Box<ConcreteType>),
    /// Heterogeneous-element double-ended queue. Storage at the
    /// `Arc<DequeData>` tier is `VecDeque<Arc<HeapValue>>` (§2.7.17
    /// Q19 deferral); the ConcreteType inner element kind is the
    /// per-element kind the producing-site classification stamps when
    /// the bytecode compiler can prove it (e.g. `Deque<int>` literal
    /// construction), else `ConcreteType::Void` placeholder.
    Deque(Box<ConcreteType>),
    /// i64-priority min-heap. i64-only at landing per ADR-006 §2.7.18
    /// (`HeapKind::PriorityQueue`); no element-kind variance. Typed-
    /// payload PriorityQueue is the Phase-2c amendment (§2.7.18
    /// "Out-of-scope") tracked separately.
    PriorityQueue,
    /// MPSC-style channel with typed payload kind. Storage at the
    /// `Arc<ChannelData>` tier holds `KindedSlot` elements (§2.7.20);
    /// the ConcreteType inner kind is the element kind for `Channel<T>`
    /// landings. `ConcreteType::Void` placeholder when the producing
    /// site can't prove the element kind.
    Channel(Box<ConcreteType>),
    /// `Mutex<T>` concurrency primitive — single typed payload protected
    /// by `Mutex<MutexInner>` per ADR-006 §2.7.25. Inner `ConcreteType`
    /// is the wrapped value's kind (`Mutex(int)` → `Mutex(I64)`); the
    /// payload is mutable at runtime and the parametric arm captures the
    /// declared inner kind for the `m.get() → T` parametric-return
    /// classifier at the §2.7.5 conduit.
    Mutex(Box<ConcreteType>),
    /// `Atomic<i64>` concurrency primitive — wraps
    /// `std::sync::atomic::AtomicI64` per ADR-006 §2.7.25. i64-only at
    /// landing per the "typed-payload deferral" precedent
    /// (W15-priority-queue i64-only, W13-hashset string-only); typed-
    /// payload `Atomic<T>` is the Phase-2c amendment tracked separately.
    Atomic,
    /// `Lazy<T>` initialize-once carrier — wraps an initializer closure
    /// + cached value slot per ADR-006 §2.7.25. Inner `ConcreteType` is
    /// the cached value's kind (the closure's return type), enabling the
    /// `l.get() → T` parametric-return classifier at the §2.7.5 conduit.
    Lazy(Box<ConcreteType>),
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
            // Round 19 S1.5 (2026-05-14): F32 and Char are 4-byte
            // scalars per the §2.7.5 amendment.
            ConcreteType::I32
            | ConcreteType::U32
            | ConcreteType::F32
            | ConcreteType::Char => 4,
            _ => 8, // f64, i64, u64, pointers, etc.
        }
    }

    /// Size in bytes when stored in a struct field (not on stack).
    #[inline]
    pub fn field_size(&self) -> usize {
        match self {
            ConcreteType::I8 | ConcreteType::U8 | ConcreteType::Bool => 1,
            ConcreteType::I16 | ConcreteType::U16 => 2,
            // Round 19 S1.5 (2026-05-14): F32 and Char are 4-byte
            // scalars per the §2.7.5 amendment.
            ConcreteType::I32
            | ConcreteType::U32
            | ConcreteType::F32
            | ConcreteType::Char => 4,
            _ => 8,
        }
    }

    /// Whether this type is a numeric type (integer or float).
    #[inline]
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            ConcreteType::F64
                | ConcreteType::F32
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
                // ── Phase 3 cluster-0 Round 11-trinity 11E ─────────────
                // All Round-11 collection / concurrency carriers are
                // heap-allocated (typed Arc<XData> per §2.7.15 /
                // §2.7.17-§2.7.20 / §2.7.25). Mirror of the existing
                // Array / HashMap heap classification.
                | ConcreteType::HashSet(_)
                | ConcreteType::Deque(_)
                | ConcreteType::PriorityQueue
                | ConcreteType::Channel(_)
                | ConcreteType::Mutex(_)
                | ConcreteType::Atomic
                | ConcreteType::Lazy(_)
        )
    }

    /// Whether this is a primitive scalar that fits in a register.
    #[inline]
    pub fn is_scalar(&self) -> bool {
        matches!(
            self,
            ConcreteType::F64
                | ConcreteType::F32
                | ConcreteType::I64
                | ConcreteType::I32
                | ConcreteType::I16
                | ConcreteType::I8
                | ConcreteType::U64
                | ConcreteType::U32
                | ConcreteType::U16
                | ConcreteType::U8
                | ConcreteType::Bool
                | ConcreteType::Char
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
            // Round 19 S1.5 (2026-05-14): F32 and Char are 4-byte
            // scalars per the §2.7.5 amendment. FieldKind has no
            // dedicated F32 / Char variants, so the bit-equivalent
            // 4-byte FieldKind::U32 is the struct-layout carrier (size
            // + alignment + load/store width all match). Semantic
            // float-vs-bits / codepoint-vs-bits distinction is preserved
            // at the NativeKind layer, NOT the struct-layout layer.
            // FieldKind cardinality extension is a follow-up sub-cluster
            // (cluster-1 hardening) if struct-field-layout typing of F32
            // / Char becomes load-bearing.
            ConcreteType::F32 | ConcreteType::Char => FieldKind::U32,
            // All reference/heap types are pointer-sized
            _ => FieldKind::Ptr,
        }
    }

    /// Generate a monomorphization key string for specialization caching.
    /// e.g., `"f64"`, `"array_i64"`, `"hashmap_string_f64"`
    pub fn mono_key(&self) -> String {
        match self {
            ConcreteType::F64 => "f64".into(),
            ConcreteType::F32 => "f32".into(),
            ConcreteType::Char => "char".into(),
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
            // ── Phase 3 cluster-0 Round 11-trinity 11E ─────────────────
            ConcreteType::HashSet(elem) => format!("hashset_{}", elem.mono_key()),
            ConcreteType::Deque(elem) => format!("deque_{}", elem.mono_key()),
            ConcreteType::PriorityQueue => "priority_queue".into(),
            ConcreteType::Channel(elem) => format!("channel_{}", elem.mono_key()),
            ConcreteType::Mutex(inner) => format!("mutex_{}", inner.mono_key()),
            ConcreteType::Atomic => "atomic".into(),
            ConcreteType::Lazy(inner) => format!("lazy_{}", inner.mono_key()),
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
            // ── Phase 3 cluster-0 Round 11-trinity 11E ─────────────────
            ConcreteType::HashSet(_) => 25,
            ConcreteType::Deque(_) => 26,
            ConcreteType::PriorityQueue => 27,
            ConcreteType::Channel(_) => 28,
            ConcreteType::Mutex(_) => 29,
            ConcreteType::Atomic => 30,
            ConcreteType::Lazy(_) => 31,
            // ── Round 19 S1.5 W12-nativekind-scalar-additions ──────────
            // (2026-05-14) — ADR-006 §2.7.5 amendment adds F32 + Char
            // as 4-byte scalar concrete types. Tags 32 and 33 allocated
            // contiguously after the Round-11 collection/concurrency arms.
            ConcreteType::F32 => 32,
            ConcreteType::Char => 33,
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
            ConcreteType::F32 => write!(f, "f32"),
            ConcreteType::Char => write!(f, "char"),
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
            // ── Phase 3 cluster-0 Round 11-trinity 11E ─────────────────
            ConcreteType::HashSet(elem) => write!(f, "HashSet<{elem}>"),
            ConcreteType::Deque(elem) => write!(f, "Deque<{elem}>"),
            ConcreteType::PriorityQueue => write!(f, "PriorityQueue"),
            ConcreteType::Channel(elem) => write!(f, "Channel<{elem}>"),
            ConcreteType::Mutex(inner) => write!(f, "Mutex<{inner}>"),
            ConcreteType::Atomic => write!(f, "Atomic"),
            ConcreteType::Lazy(inner) => write!(f, "Lazy<{inner}>"),
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
            // ── Phase 3 cluster-0 Round 11-trinity 11E ─────────────────
            ConcreteType::HashSet(Box::new(ConcreteType::String)),
            ConcreteType::Deque(Box::new(ConcreteType::I64)),
            ConcreteType::PriorityQueue,
            ConcreteType::Channel(Box::new(ConcreteType::I64)),
            ConcreteType::Mutex(Box::new(ConcreteType::I64)),
            ConcreteType::Atomic,
            ConcreteType::Lazy(Box::new(ConcreteType::I64)),
            // ── Round 19 S1.5 W12-nativekind-scalar-additions ─────────
            ConcreteType::F32,
            ConcreteType::Char,
        ];
        let tags: Vec<u8> = types.iter().map(|t| t.type_tag()).collect();
        let unique: std::collections::HashSet<u8> = tags.iter().copied().collect();
        assert_eq!(tags.len(), unique.len(), "type tags must be unique");
    }

    /// Round 19 S1.5 (2026-05-14): F32 + Char additions ride the same
    /// scalar dispatch shape as the existing 4-byte scalars (I32 / U32).
    #[test]
    fn test_round_19_f32_char_scalars() {
        assert_eq!(ConcreteType::F32.mono_key(), "f32");
        assert_eq!(ConcreteType::Char.mono_key(), "char");
        assert_eq!(format!("{}", ConcreteType::F32), "f32");
        assert_eq!(format!("{}", ConcreteType::Char), "char");
        // 4-byte alignment + field-size match I32 / U32.
        assert_eq!(ConcreteType::F32.alignment(), 4);
        assert_eq!(ConcreteType::F32.field_size(), 4);
        assert_eq!(ConcreteType::Char.alignment(), 4);
        assert_eq!(ConcreteType::Char.field_size(), 4);
        // Both are scalar; F32 is numeric (float-family), Char is NOT
        // numeric (UTF-32 codepoint, not a numeric type).
        assert!(ConcreteType::F32.is_scalar());
        assert!(ConcreteType::Char.is_scalar());
        assert!(ConcreteType::F32.is_numeric());
        assert!(!ConcreteType::Char.is_numeric());
        assert!(!ConcreteType::F32.is_integer());
        assert!(!ConcreteType::Char.is_integer());
        // Neither is heap-allocated.
        assert!(!ConcreteType::F32.is_heap());
        assert!(!ConcreteType::Char.is_heap());
    }

    #[test]
    fn test_round_11_collection_concurrency_arms_mono_key_and_display() {
        // Phase 3 cluster-0 Round 11-trinity 11E: collection/concurrency
        // arms round-trip through mono_key and Display.
        let hs = ConcreteType::HashSet(Box::new(ConcreteType::String));
        assert_eq!(hs.mono_key(), "hashset_string");
        assert_eq!(format!("{hs}"), "HashSet<string>");

        let dq = ConcreteType::Deque(Box::new(ConcreteType::I64));
        assert_eq!(dq.mono_key(), "deque_i64");
        assert_eq!(format!("{dq}"), "Deque<int>");

        let pq = ConcreteType::PriorityQueue;
        assert_eq!(pq.mono_key(), "priority_queue");
        assert_eq!(format!("{pq}"), "PriorityQueue");

        let ch = ConcreteType::Channel(Box::new(ConcreteType::I64));
        assert_eq!(ch.mono_key(), "channel_i64");
        assert_eq!(format!("{ch}"), "Channel<int>");

        let mx = ConcreteType::Mutex(Box::new(ConcreteType::I64));
        assert_eq!(mx.mono_key(), "mutex_i64");
        assert_eq!(format!("{mx}"), "Mutex<int>");

        let at = ConcreteType::Atomic;
        assert_eq!(at.mono_key(), "atomic");
        assert_eq!(format!("{at}"), "Atomic");

        let lz = ConcreteType::Lazy(Box::new(ConcreteType::Bool));
        assert_eq!(lz.mono_key(), "lazy_bool");
        assert_eq!(format!("{lz}"), "Lazy<bool>");
    }

    #[test]
    fn test_round_11_arms_are_heap() {
        // All 7 new arms are heap-allocated typed-Arc carriers per
        // ADR-006 §2.7.15 / §2.7.17-§2.7.20 / §2.7.25.
        assert!(ConcreteType::HashSet(Box::new(ConcreteType::String)).is_heap());
        assert!(ConcreteType::Deque(Box::new(ConcreteType::I64)).is_heap());
        assert!(ConcreteType::PriorityQueue.is_heap());
        assert!(ConcreteType::Channel(Box::new(ConcreteType::I64)).is_heap());
        assert!(ConcreteType::Mutex(Box::new(ConcreteType::I64)).is_heap());
        assert!(ConcreteType::Atomic.is_heap());
        assert!(ConcreteType::Lazy(Box::new(ConcreteType::I64)).is_heap());

        // None are scalar / numeric / integer.
        assert!(!ConcreteType::HashSet(Box::new(ConcreteType::String)).is_scalar());
        assert!(!ConcreteType::Mutex(Box::new(ConcreteType::I64)).is_numeric());
        assert!(!ConcreteType::Atomic.is_integer());
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
