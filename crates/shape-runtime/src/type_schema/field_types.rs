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
    /// Optional value `Option<T>` with concrete inner `FieldType`.
    ///
    /// v0.3 Phase 4b Round 5b W17.2-B — per audit §4.D.1 + §9.B.1 (a)
    /// PROPAGATE supervisor ratify 2026-05-19. Replaces the deleted
    /// `Option<_> => FieldType::Any` fallback that erased known
    /// structure at `semantic_to_field_type`. Slot storage stays
    /// NaN-boxed at the bits level via the parallel-`field_kinds`
    /// track at storage construction time (per ADR-006 §2.7.7 / Q9 +
    /// §2.7.26): the kind for `None` is `NativeKind::Null`; for
    /// `Some(x)` the inner `T`'s kind. Snapshot/wire serialization
    /// extends naturally via serde's enum-discriminator.
    ///
    /// Mirrors the §4.D.14 enum-payload exception (which already uses
    /// the parallel-`field_kinds` track at storage time per §2.7.26).
    Option(Box<FieldType>),
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
            FieldType::Option(inner) => write!(f, "Option<{}>", inner),
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
            // Option<T>: NaN-boxed slot at bits level; the parallel
            // `field_kinds` track carries the per-value kind (Null for
            // None, inner T's kind for Some(x)) per ADR-006 §2.7.7.
            FieldType::Option(_) => 8,
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
            // Option<T>: the schema layer's discriminator is `Option`,
            // but slot storage carries the per-value kind via the
            // parallel `field_kinds` track at storage construction
            // time (ADR-006 §2.7.7 / Q9 + §2.7.26). The schema-level
            // projection refuses statically — runtime callers must
            // read the per-slot kind, not the static FieldType kind.
            // Same refusal-shape as `Any` (which is what this variant
            // SUBSUMES post-W17.2-B per audit §4.D.1 disposition (a)).
            Self::Option(_) => Err(FieldKindError::AnyTypeNotStrictlyTyped),
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

/// Convert SemanticType to FieldType for schema creation.
///
/// v0.3 Phase 4b Round 5b W17.2-B (audit §4.D.1 + §4.D.2 + §4.D.9 +
/// §9.B.1 (a) PROPAGATE supervisor ratify 2026-05-19). Per binding user
/// 2026-05-18 ruling "after the pass, any needs to be gone": this
/// function returns `FieldType::Any` from ZERO producer sites. The
/// `is_optional` + `SemanticType::Option(_)` fallbacks (formerly
/// returning `FieldType::Any` for "Optional values use NaN boxing")
/// now route through the explicit `FieldType::Option(Box<FieldType>)`
/// variant — slot storage stays NaN-boxed at the bits level via the
/// parallel `field_kinds` track at storage construction time per
/// ADR-006 §2.7.7 / Q9 + §2.7.26 (W17-comptime-vm-dispatch).
///
/// All SemanticType variants are explicitly enumerated; the deleted
/// `_ => FieldType::Any` catch-all per audit §4.D.2 is replaced by
/// explicit per-variant arms. Inference-tier variants
/// (TypeVar/Never/Void/Function) reach this function ONLY when
/// upstream inference left a soundness gap; they surface via
/// `unreachable!()` rather than silently emitting an Any fallback,
/// per the audit §4.D.2 (b) ERROR disposition. Named/Generic route to
/// `FieldType::Object(name)` (preserves W15.2-LANG-8 schema-lookup
/// discipline); Struct/Enum route to `FieldType::Object(name)` via
/// their `.name` projection; Ref/RefMut transparently unwrap to
/// inner kind. Result<T, E> routes to `FieldType::Object("Result")`
/// — the schema-side discriminator preserves the "Result" container
/// shape for the post-inference verify pass to consult against
/// helpers.rs:4901 generic-container exception territory (W17.2-C).
pub(crate) fn semantic_to_field_type(
    semantic: &crate::type_system::SemanticType,
    is_optional: bool,
) -> FieldType {
    use crate::type_system::SemanticType;

    // Optional field at SCHEMA-declaration time (the `field: T?` syntax
    // — `cf.optional = true`). PROPAGATE-rebuild per audit §4.D.1
    // disposition (a): wrap the inner FieldType in
    // `FieldType::Option(Box<FieldType>)` rather than erasing to Any.
    // The inner kind threads through to the slot's parallel-field_kinds
    // track at TypedObject construction time per ADR-006 §2.7.26.
    if is_optional {
        return FieldType::Option(Box::new(semantic_to_field_type(semantic, false)));
    }

    match semantic {
        // === Primitives (statically resolved at the producer site) ===
        SemanticType::Number => FieldType::F64,
        SemanticType::Integer => FieldType::I64,
        SemanticType::Bool => FieldType::Bool,
        SemanticType::String => FieldType::String,

        // === Generic Containers (semantic-tier inner type known) ===
        SemanticType::Array(elem) => {
            FieldType::Array(Box::new(semantic_to_field_type(elem, false)))
        }
        // §4.D.1 (a) PROPAGATE — Option<T> with concrete inner T IS
        // fully resolved at annotation-lowering time per supervisor
        // 2026-05-19; the schema-side discriminator preserves the
        // inner kind explicitly. Subsumes §4.D.9 `Literal::None` site
        // (None lowers via this arm at the bidirectional-inference
        // narrowing call site at compiler/expressions/collections.rs).
        SemanticType::Option(inner) => {
            FieldType::Option(Box::new(semantic_to_field_type(inner, false)))
        }
        // Result<T, E> is a discriminated union at the schema layer —
        // it's a CLOSURE-WAVE carrier-tier exception class that the
        // post-inference pass's permanent whitelist may absorb (v0.4
        // candidate W17.3 territory per audit §8 closure-wave plan).
        // At this lowering site, the inner Ok-type is preserved via
        // FieldType::Object("Result") for schema-lookup, mirroring the
        // helpers.rs:4901 generic-container exception list narrowing
        // (W17.2-C territory). The Err type erases through the
        // marshal-layer's Json fallback per §4.D.7.
        SemanticType::Result { .. } => FieldType::Object("Result".to_string()),

        // === User-Defined Types (resolved at annotation site) ===
        SemanticType::Struct { name, .. } => FieldType::Object(name.clone()),
        SemanticType::Enum { name, .. } => FieldType::Object(name.clone()),

        // === Named type references ===
        SemanticType::Named(name) if name == "Decimal" => FieldType::Decimal,
        SemanticType::Named(name) => FieldType::Object(name.clone()),

        // === Generic instantiation MyType<A, B> — schema-lookup by
        //     erased name; the args are projected at type-resolution
        //     time before reaching the schema layer.
        SemanticType::Generic { name, .. } => FieldType::Object(name.clone()),

        // === Reference Types — schema-layer treats refs as
        //     transparent-projection: the schema stores the underlying
        //     type. Mirrors the borrow-solver's "&T → T at storage" rule.
        SemanticType::Ref(inner) | SemanticType::RefMut(inner) => {
            semantic_to_field_type(inner, false)
        }

        // === Type System Internals — UNREACHABLE at schema lowering.
        //     Per audit §4.D.2 (b) ERROR disposition: "any unreachable
        //     arm gets `compile_error!()` or `unreachable!()` at the
        //     lowering layer". TypeVars MUST be resolved by inference
        //     before reaching schema construction; if they aren't,
        //     that's a soundness gap upstream — surface here rather
        //     than silently emitting an Any fallback.
        //
        //     If any of these arms fire in practice, the surface is
        //     a structured panic that points to the inference-tier
        //     gap. The post_inference_verify.rs pass at W17.2-A
        //     catches the SCHEMA-level Any cases; this panic catches
        //     the SEMANTIC-tier unresolved cases that should never
        //     reach schema construction.
        SemanticType::TypeVar(_) => unreachable!(
            "semantic_to_field_type: TypeVar reached schema lowering — \
             upstream inference left a type variable unresolved at \
             post-inference schema construction. Per ADR-006 §2.7.5 \
             producer-side stamp + audit §4.D.2 (b) ERROR disposition: \
             this is a soundness gap in the inference layer, not a \
             schema-side fallback target."
        ),

        // === Special Types — Never/Void/Function don't map to slot
        //     storage; they reach this function only via pathological
        //     paths (e.g. unresolved closure return type bubbling into
        //     a struct field). Same `unreachable!()` disposition as
        //     TypeVar per audit §4.D.2 — these should be caught at
        //     inference time, not papered over with Any.
        SemanticType::Never => unreachable!(
            "semantic_to_field_type: Never type reached schema lowering — \
             a bottom-type field has no storage representation. Audit §4.D.2."
        ),
        SemanticType::Void => unreachable!(
            "semantic_to_field_type: Void type reached schema lowering — \
             a unit-type field should be elided at the inference layer. \
             Audit §4.D.2."
        ),
        SemanticType::Function(_) => unreachable!(
            "semantic_to_field_type: Function type reached schema lowering — \
             closures/function references live in HeapKind::Closure slots, \
             not as schema-declared field types. Audit §4.D.2."
        ),
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
        // W17.2-B: Option<T> NaN-boxed slot; per-value kind from
        // parallel-`field_kinds` track per ADR-006 §2.7.7.
        assert_eq!(FieldType::Option(Box::new(FieldType::F64)).size(), 8);
        assert_eq!(FieldType::Option(Box::new(FieldType::I64)).size(), 8);
        assert_eq!(
            FieldType::Option(Box::new(FieldType::Object("X".to_string()))).size(),
            8
        );
    }

    #[test]
    fn test_field_type_alignment() {
        assert_eq!(FieldType::F64.alignment(), 8);
        assert_eq!(FieldType::I64.alignment(), 8);
        assert_eq!(FieldType::Bool.alignment(), 8);
        assert_eq!(FieldType::Option(Box::new(FieldType::F64)).alignment(), 8);
    }

    #[test]
    fn test_field_def_creation() {
        let field = FieldDef::new("test", FieldType::F64, 16, 2);
        assert_eq!(field.name, "test");
        assert_eq!(field.field_type, FieldType::F64);
        assert_eq!(field.offset, 16);
        assert_eq!(field.index, 2);
    }

    // ----- W17.2-B FieldType::Option PROPAGATE regression tests -----
    //
    // Per audit §4.D.1 + §9.B.1 (a) supervisor ratify 2026-05-19:
    // `semantic_to_field_type` returns `FieldType::Any` from ZERO
    // producer callsites for Option / is_optional / Array<Option<T>> /
    // recursive nesting. Inner kind threads through the schema layer
    // via the new `FieldType::Option(Box<FieldType>)` variant.

    /// §4.D.1 — `is_optional = true` with concrete inner: PROPAGATE
    /// inner kind via `FieldType::Option(Box<inner>)`. Was Any.
    #[test]
    fn test_semantic_to_field_type_optional_int() {
        use crate::type_system::SemanticType;
        let ft = semantic_to_field_type(&SemanticType::Integer, true);
        assert_eq!(ft, FieldType::Option(Box::new(FieldType::I64)));
    }

    /// §4.D.1 — `is_optional = true` with concrete number inner.
    #[test]
    fn test_semantic_to_field_type_optional_number() {
        use crate::type_system::SemanticType;
        let ft = semantic_to_field_type(&SemanticType::Number, true);
        assert_eq!(ft, FieldType::Option(Box::new(FieldType::F64)));
    }

    /// §4.D.1 — `is_optional = true` with concrete object inner.
    #[test]
    fn test_semantic_to_field_type_optional_object() {
        use crate::type_system::SemanticType;
        let ft = semantic_to_field_type(
            &SemanticType::Named("Candle".to_string()),
            true,
        );
        assert_eq!(
            ft,
            FieldType::Option(Box::new(FieldType::Object("Candle".to_string())))
        );
    }

    /// §4.D.1 — `SemanticType::Option(inner)` arm without
    /// `is_optional`: same PROPAGATE shape (the audit-binding
    /// "all SemanticType variants explicitly handled" close gate).
    #[test]
    fn test_semantic_to_field_type_option_variant_int() {
        use crate::type_system::SemanticType;
        let ft = semantic_to_field_type(
            &SemanticType::Option(Box::new(SemanticType::Integer)),
            false,
        );
        assert_eq!(ft, FieldType::Option(Box::new(FieldType::I64)));
    }

    /// §4.D.1 — `SemanticType::Option(Bool)` propagates inner Bool kind.
    #[test]
    fn test_semantic_to_field_type_option_variant_bool() {
        use crate::type_system::SemanticType;
        let ft = semantic_to_field_type(
            &SemanticType::Option(Box::new(SemanticType::Bool)),
            false,
        );
        assert_eq!(ft, FieldType::Option(Box::new(FieldType::Bool)));
    }

    /// §4.D.1 — Array<Option<T>> threads Option through Array element kind.
    #[test]
    fn test_semantic_to_field_type_array_of_option() {
        use crate::type_system::SemanticType;
        let inner = SemanticType::Array(Box::new(SemanticType::Option(Box::new(
            SemanticType::String,
        ))));
        let ft = semantic_to_field_type(&inner, false);
        assert_eq!(
            ft,
            FieldType::Array(Box::new(FieldType::Option(Box::new(FieldType::String))))
        );
    }

    /// §4.D.2 close-gate: `semantic_to_field_type` returns
    /// `FieldType::Any` from ZERO producer callsites for the
    /// previously-Any-fallback inputs. Asserts Any is NOT returned
    /// for: is_optional + Integer / Option(Integer) / Array<Option<T>>.
    #[test]
    fn test_semantic_to_field_type_returns_no_any_for_option_inputs() {
        use crate::type_system::SemanticType;

        // is_optional=true + Integer (was Any)
        let ft1 = semantic_to_field_type(&SemanticType::Integer, true);
        assert!(
            !matches!(ft1, FieldType::Any),
            "is_optional=true must not return Any; got {:?}",
            ft1
        );

        // Option(Bool) (was Any)
        let ft2 = semantic_to_field_type(
            &SemanticType::Option(Box::new(SemanticType::Bool)),
            false,
        );
        assert!(
            !matches!(ft2, FieldType::Any),
            "SemanticType::Option(_) must not return Any; got {:?}",
            ft2
        );

        // Array<Option<Integer>> nests (no inner Any)
        let ft3 = semantic_to_field_type(
            &SemanticType::Array(Box::new(SemanticType::Option(Box::new(
                SemanticType::Integer,
            )))),
            false,
        );
        let unwrapped_array = match ft3 {
            FieldType::Array(inner) => *inner,
            other => panic!("expected Array, got {:?}", other),
        };
        assert!(
            !matches!(unwrapped_array, FieldType::Any),
            "Array<Option<Integer>> inner must not be Any; got {:?}",
            unwrapped_array
        );
    }

    /// §4.D.1 — concrete-typed inputs preserve their concrete kind
    /// (regression: PROPAGATE rebuild does not corrupt non-Option paths).
    #[test]
    fn test_semantic_to_field_type_concrete_preserved() {
        use crate::type_system::SemanticType;
        assert_eq!(
            semantic_to_field_type(&SemanticType::Integer, false),
            FieldType::I64
        );
        assert_eq!(
            semantic_to_field_type(&SemanticType::Number, false),
            FieldType::F64
        );
        assert_eq!(
            semantic_to_field_type(&SemanticType::Bool, false),
            FieldType::Bool
        );
        assert_eq!(
            semantic_to_field_type(&SemanticType::String, false),
            FieldType::String
        );
        assert_eq!(
            semantic_to_field_type(&SemanticType::Named("Decimal".to_string()), false),
            FieldType::Decimal
        );
    }

    /// `Display` for the new variant — `Option<int>` shape.
    #[test]
    fn test_field_type_option_display() {
        let ft = FieldType::Option(Box::new(FieldType::I64));
        assert_eq!(format!("{}", ft), "Option<int>");
        let nested = FieldType::Option(Box::new(FieldType::Object("Candle".to_string())));
        assert_eq!(format!("{}", nested), "Option<Candle>");
    }

    /// `to_native_kind()` for Option<T> refuses statically — slot kind
    /// lives in the parallel-`field_kinds` track per ADR-006 §2.7.7.
    /// Same refusal-shape as `Any` (which the variant SUBSUMES).
    #[test]
    fn test_field_type_option_to_native_kind_refuses() {
        let ft = FieldType::Option(Box::new(FieldType::I64));
        assert!(matches!(
            ft.to_native_kind(),
            Err(FieldKindError::AnyTypeNotStrictlyTyped)
        ));
    }
}
