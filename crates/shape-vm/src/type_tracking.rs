//! Type Tracking for Bytecode Compiler
//!
//! This module tracks known type information during compilation to enable
//! type-specialized code generation. When a variable's type is known at
//! compile time, the compiler can emit optimized opcodes for field access.
//!
//! # How Types Become Known
//!
//! Types are known in these situations:
//! - Explicit type annotation: `let x: Candle = ...`
//! - Constructor call: `let x = Candle { ... }`
//! - Object literal: `let x = { a: 1, b: 2 }` (inline struct type)
//! - Function with declared return type: `let x = get_candle()`
//!
//! # Usage
//!
//! The compiler uses this to emit typed field opcodes for dot access:
//! - `GetFieldTyped` (specialized): Direct slot access by precomputed offset
//! - `SetFieldTyped` (specialized): Direct slot update by precomputed offset
//! Generic `GetProp`/`SetProp` are reserved for non-dot operations (index/slice).
//!
//! # Storage Type Hints
//!
//! For JIT optimization, we track storage types:
//! - `StorageHint::NullableFloat64`: Option<f64> uses NaN sentinel
//! - `StorageHint::Float64`: Plain f64, no nullability
//! - `StorageHint::Unknown`: Type not determined at compile time

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use shape_ast::ast::TypeAnnotation;
use shape_runtime::type_schema::{FieldType, SchemaId, TypeSchema, TypeSchemaRegistry};
use shape_runtime::type_system::{BuiltinTypes, StorageType};
use shape_value::v2::struct_layout::{FieldKind, StructLayout};
use shape_value::ValueWordExt;

/// Numeric type known at compile time for typed opcode emission.
///
/// When the compiler can determine the numeric subtype of an expression,
/// it emits typed opcodes (e.g., `MulInt` instead of `Mul`) that skip
/// runtime type dispatch entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericType {
    /// Integer (i64) — the default integer type
    Int,
    /// Width-specific integer (i8, u8, i16, u16, i32, u32, u64)
    IntWidth(shape_ast::IntWidth),
    /// Floating point (f64)
    Number,
    /// Exact decimal (rust_decimal::Decimal)
    Decimal,
}

/// Counter for generating unique inline object type names
static INLINE_OBJECT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Describes the storage kind for a single local/parameter slot in a frame.
///
/// Used by the JIT and VM to generate more efficient code by knowing
/// the actual storage representation at compile time.
///
/// This was previously named `StorageHint`; the alias is kept for
/// backwards compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SlotKind {
    /// Plain f64 value (direct float operations)
    Float64,
    /// Nullable f64 using NaN sentinel (Option<f64>)
    /// IEEE 754: NaN + x = NaN, so null propagates automatically
    NullableFloat64,
    /// Plain i8 value
    Int8,
    /// Nullable i8 value
    NullableInt8,
    /// Plain u8 value
    UInt8,
    /// Nullable u8 value
    NullableUInt8,
    /// Plain i16 value
    Int16,
    /// Nullable i16 value
    NullableInt16,
    /// Plain u16 value
    UInt16,
    /// Nullable u16 value
    NullableUInt16,
    /// Plain i32 value
    Int32,
    /// Nullable i32 value
    NullableInt32,
    /// Plain u32 value
    UInt32,
    /// Nullable u32 value
    NullableUInt32,
    /// Plain i64 value
    Int64,
    /// Nullable i64 value
    NullableInt64,
    /// Plain u64 value
    UInt64,
    /// Nullable u64 value
    NullableUInt64,
    /// Plain isize value
    IntSize,
    /// Nullable isize value
    NullableIntSize,
    /// Plain usize value
    UIntSize,
    /// Nullable usize value
    NullableUIntSize,
    /// Boolean value
    Bool,
    /// String reference
    String,
    /// Dynamically-typed value: the raw u64 bits are a valid interpreter value.
    /// Used for boxed locals and operand stack entries in precise deopt metadata.
    /// The VM unmarshals these via direct transmute (zero-cost passthrough).
    Dynamic,
    /// Type not determined at compile time (falls back to dynamic dispatch).
    /// Should NOT appear in precise deopt metadata — use Dynamic instead.
    /// Reserved for truly uninitialized/unresolved slots.
    Unknown,
}

/// Backwards-compatible alias. Prefer `SlotKind` in new code.
pub type StorageHint = SlotKind;

impl Default for SlotKind {
    fn default() -> Self {
        SlotKind::Unknown
    }
}

impl From<StorageType> for SlotKind {
    /// Convert from runtime StorageType to JIT StorageHint
    fn from(st: StorageType) -> Self {
        Self::from_storage_type(&st)
    }
}

impl SlotKind {
    /// Convert from runtime StorageType
    ///
    /// Maps semantic storage types to JIT optimization hints:
    /// - Primitive types map directly
    /// - NullableFloat64 enables NaN sentinel optimization
    /// - Complex types fall back to boxed representation
    pub fn from_storage_type(st: &StorageType) -> Self {
        match st {
            // Direct mappings for primitives
            StorageType::Float64 => StorageHint::Float64,
            StorageType::Int64 => StorageHint::Int64,
            StorageType::Bool => StorageHint::Bool,
            StorageType::String => StorageHint::String,

            // Nullable types with optimized storage
            StorageType::NullableFloat64 => StorageHint::NullableFloat64,
            StorageType::NullableInt64 => StorageHint::NullableInt64,
            StorageType::NullableBool => StorageHint::Bool, // 3-state in Boxed

            // Complex types use boxed representation
            StorageType::Array(_)
            | StorageType::Table { .. }
            | StorageType::Object
            | StorageType::Result { .. }
            | StorageType::TaggedUnion { .. }
            | StorageType::Function
            | StorageType::Struct(_)
            | StorageType::Dynamic => StorageHint::Unknown,
        }
    }

    #[inline]
    pub fn is_integer(self) -> bool {
        matches!(
            self,
            Self::Int8
                | Self::UInt8
                | Self::Int16
                | Self::UInt16
                | Self::Int32
                | Self::UInt32
                | Self::Int64
                | Self::UInt64
                | Self::IntSize
                | Self::UIntSize
        )
    }

    #[inline]
    pub fn is_nullable_integer(self) -> bool {
        matches!(
            self,
            Self::NullableInt8
                | Self::NullableUInt8
                | Self::NullableInt16
                | Self::NullableUInt16
                | Self::NullableInt32
                | Self::NullableUInt32
                | Self::NullableInt64
                | Self::NullableUInt64
                | Self::NullableIntSize
                | Self::NullableUIntSize
        )
    }

    #[inline]
    pub fn is_integer_family(self) -> bool {
        self.is_integer() || self.is_nullable_integer()
    }

    #[inline]
    pub fn is_default_int_family(self) -> bool {
        matches!(self, Self::Int64 | Self::NullableInt64)
    }

    #[inline]
    pub fn is_float_family(self) -> bool {
        matches!(self, Self::Float64 | Self::NullableFloat64)
    }

    #[inline]
    pub fn is_numeric_family(self) -> bool {
        self.is_integer_family() || self.is_float_family()
    }

    #[inline]
    pub fn is_pointer_sized_integer(self) -> bool {
        matches!(
            self,
            Self::IntSize | Self::UIntSize | Self::NullableIntSize | Self::NullableUIntSize
        )
    }

    #[inline]
    pub fn is_signed_integer(self) -> Option<bool> {
        if matches!(
            self,
            Self::Int8
                | Self::NullableInt8
                | Self::Int16
                | Self::NullableInt16
                | Self::Int32
                | Self::NullableInt32
                | Self::Int64
                | Self::NullableInt64
                | Self::IntSize
                | Self::NullableIntSize
        ) {
            Some(true)
        } else if matches!(
            self,
            Self::UInt8
                | Self::NullableUInt8
                | Self::UInt16
                | Self::NullableUInt16
                | Self::UInt32
                | Self::NullableUInt32
                | Self::UInt64
                | Self::NullableUInt64
                | Self::UIntSize
                | Self::NullableUIntSize
        ) {
            Some(false)
        } else {
            None
        }
    }

    #[inline]
    pub fn integer_bit_width(self) -> Option<u16> {
        match self {
            Self::Int8 | Self::UInt8 | Self::NullableInt8 | Self::NullableUInt8 => Some(8),
            Self::Int16 | Self::UInt16 | Self::NullableInt16 | Self::NullableUInt16 => Some(16),
            Self::Int32 | Self::UInt32 | Self::NullableInt32 | Self::NullableUInt32 => Some(32),
            Self::Int64 | Self::UInt64 | Self::NullableInt64 | Self::NullableUInt64 => Some(64),
            Self::IntSize | Self::UIntSize | Self::NullableIntSize | Self::NullableUIntSize => {
                Some(usize::BITS as u16)
            }
            _ => None,
        }
    }

    #[inline]
    pub fn non_nullable(self) -> Self {
        match self {
            Self::NullableFloat64 => Self::Float64,
            Self::NullableInt8 => Self::Int8,
            Self::NullableUInt8 => Self::UInt8,
            Self::NullableInt16 => Self::Int16,
            Self::NullableUInt16 => Self::UInt16,
            Self::NullableInt32 => Self::Int32,
            Self::NullableUInt32 => Self::UInt32,
            Self::NullableInt64 => Self::Int64,
            Self::NullableUInt64 => Self::UInt64,
            Self::NullableIntSize => Self::IntSize,
            Self::NullableUIntSize => Self::UIntSize,
            other => other,
        }
    }

    #[inline]
    pub fn with_nullability(self, nullable: bool) -> Self {
        if !nullable {
            return self.non_nullable();
        }
        match self.non_nullable() {
            Self::Float64 => Self::NullableFloat64,
            Self::Int8 => Self::NullableInt8,
            Self::UInt8 => Self::NullableUInt8,
            Self::Int16 => Self::NullableInt16,
            Self::UInt16 => Self::NullableUInt16,
            Self::Int32 => Self::NullableInt32,
            Self::UInt32 => Self::NullableUInt32,
            Self::Int64 => Self::NullableInt64,
            Self::UInt64 => Self::NullableUInt64,
            Self::IntSize => Self::NullableIntSize,
            Self::UIntSize => Self::NullableUIntSize,
            other => other,
        }
    }

    pub fn combine_integer_hints(lhs: Self, rhs: Self) -> Option<Self> {
        let lhs_bits = lhs.integer_bit_width()?;
        let rhs_bits = rhs.integer_bit_width()?;
        let bits = lhs_bits.max(rhs_bits);
        let signed = lhs.is_signed_integer()? || rhs.is_signed_integer()?;
        let nullable = lhs.is_nullable_integer() || rhs.is_nullable_integer();
        let keep_pointer_size = bits == usize::BITS as u16
            && (lhs.is_pointer_sized_integer() || rhs.is_pointer_sized_integer());
        let base = if keep_pointer_size {
            if signed {
                Self::IntSize
            } else {
                Self::UIntSize
            }
        } else {
            match (bits, signed) {
                (8, true) => Self::Int8,
                (8, false) => Self::UInt8,
                (16, true) => Self::Int16,
                (16, false) => Self::UInt16,
                (32, true) => Self::Int32,
                (32, false) => Self::UInt32,
                (64, true) => Self::Int64,
                (64, false) => Self::UInt64,
                _ => return None,
            }
        };
        Some(base.with_nullability(nullable))
    }
}

/// Typed frame layout metadata.
///
/// A `FrameDescriptor` describes the storage layout for every local slot
/// (parameters + locals) in a single function or top-level frame.  The JIT
/// and VM use this to allocate registers / stack space with correct widths
/// and to skip NaN-boxing for slots whose type is statically known.
///
/// This is the canonical replacement for the loose `Vec<StorageHint>` arrays
/// that were previously threaded through `BytecodeProgram` and `Function`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameDescriptor {
    /// One entry per local slot (index 0 = first param or local).
    /// A `Boxed` entry means the slot stores a generic NaN-boxed value.
    pub slots: Vec<SlotKind>,

    /// Return type kind for the function.
    ///
    /// When present and not `Unknown`, the JIT boundary ABI uses this to
    /// unmarshal the return value from JIT-compiled code back into the
    /// correct `ValueWord` representation.
    #[serde(default)]
    pub return_kind: SlotKind,
}

impl FrameDescriptor {
    /// Create an empty descriptor (all slots will be Boxed by default).
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            return_kind: SlotKind::Unknown,
        }
    }

    /// Create a descriptor with `n` slots, all initialised to `SlotKind::Unknown`.
    pub fn with_unknown_slots(n: usize) -> Self {
        Self {
            slots: vec![SlotKind::Unknown; n],
            return_kind: SlotKind::Unknown,
        }
    }

    /// Build a descriptor from an existing `Vec<SlotKind>` (or `Vec<StorageHint>`).
    pub fn from_slots(slots: Vec<SlotKind>) -> Self {
        Self {
            slots,
            return_kind: SlotKind::Unknown,
        }
    }

    /// Number of slots described.
    #[inline]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the descriptor is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Get the kind of a specific slot.  Returns `Boxed` for out-of-range indices.
    #[inline]
    pub fn slot(&self, index: usize) -> SlotKind {
        self.slots.get(index).copied().unwrap_or(SlotKind::Unknown)
    }

    /// Returns `true` if every slot is `Unknown` (i.e. no specialization).
    pub fn is_all_unknown(&self) -> bool {
        self.slots.iter().all(|s| *s == SlotKind::Unknown)
    }
}

/// The kind of variable: regular value, typed table, row view, or column.
///
/// Replaces the old `is_datatable` / `is_row_view` / `is_column` boolean flags,
/// which were mutually exclusive but had no compiler enforcement.
#[derive(Debug, Clone, PartialEq)]
pub enum VariableKind {
    /// Regular value (struct, primitive, object, etc.)
    Value,
    /// A DataTable with known row schema — Table<T>.
    /// Closure methods (filter/map/etc.) propagate schema to row params.
    Table { element_type: String },
    /// A typed row from an Arrow DataTable — Row<T>.
    /// Field access emits LoadColF64/I64/Bool/Str instead of GetProp.
    RowView { element_type: String },
    /// A typed column from an Arrow DataTable — Column<T>.
    Column {
        element_type: String,
        column_type: String,
    },
    /// An indexed table — Indexed<T> with a designated index column.
    /// Only Indexed tables can use resample/between operations.
    Indexed {
        element_type: String,
        index_column: String,
    },
}

/// Source-level ownership class for a binding slot.
///
/// This tracks how the binding was declared, independent of the value's type.
/// Later storage planning uses this to decide whether a slot can stay direct,
/// must allow aliasing, or should preserve reference representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingOwnershipClass {
    /// `let` — immutable owned binding.
    OwnedImmutable,
    /// `let mut` — mutable owned binding.
    OwnedMutable,
    /// `var` — flexible/aliasable binding whose storage is chosen later.
    Flexible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aliasability {
    /// Single owner, no aliasing possible.
    Unique,
    /// Shared via immutable references only.
    SharedImmutable,
    /// Shared with potential mutation (var semantics).
    SharedMutable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationCapability {
    /// Cannot be mutated (`let`).
    Immutable,
    /// Mutable by single owner (`let mut`).
    LocalMutable,
    /// Mutable with shared access (`var` captured/aliased).
    SharedMutable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EscapeStatus {
    /// Stays within declaring scope.
    Local,
    /// Captured by a closure.
    Captured,
    /// Escapes declaring function (returned, stored in module state).
    Escaped,
}

/// Planned runtime storage strategy for a binding slot.
///
/// `Deferred` is the initial state for ordinary bindings until a later planner
/// decides whether the slot can stay direct or must be upgraded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingStorageClass {
    Deferred,
    Direct,
    UniqueHeap,
    SharedCow,
    Reference,
}

/// Ownership/storage metadata for a binding slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingSemantics {
    pub ownership_class: BindingOwnershipClass,
    pub storage_class: BindingStorageClass,
    pub aliasability: Aliasability,
    pub mutation_capability: MutationCapability,
    pub escape_status: EscapeStatus,
}

impl BindingSemantics {
    pub const fn deferred(ownership_class: BindingOwnershipClass) -> Self {
        Self {
            ownership_class,
            storage_class: BindingStorageClass::Deferred,
            aliasability: Aliasability::Unique,
            mutation_capability: match ownership_class {
                BindingOwnershipClass::OwnedImmutable => MutationCapability::Immutable,
                BindingOwnershipClass::OwnedMutable => MutationCapability::LocalMutable,
                BindingOwnershipClass::Flexible => MutationCapability::SharedMutable,
            },
            escape_status: EscapeStatus::Local,
        }
    }
}

/// Type information for a variable
#[derive(Debug, Clone)]
pub struct VariableTypeInfo {
    /// Schema ID if type is known and registered
    pub schema_id: Option<SchemaId>,
    /// Type name (e.g., "Candle", "Point")
    pub type_name: Option<String>,
    /// Whether the type is definitely known (vs inferred/uncertain)
    pub is_definite: bool,
    /// Storage hint for JIT optimization
    pub storage_hint: StorageHint,
    /// Preserved concrete numeric runtime type (e.g. "i16", "u8", "f32", "i64")
    /// derived from source annotations.
    pub concrete_numeric_type: Option<String>,
    /// What kind of variable this is (value, table, row view, column)
    pub kind: VariableKind,
    /// v2: For typed arrays, the element's FieldKind (enables typed array codegen)
    pub v2_array_element_kind: Option<FieldKind>,
    /// v2: For typed structs, the SchemaId referencing a StructLayout in TypeTracker
    pub v2_struct_layout: Option<SchemaId>,
}

impl VariableTypeInfo {
    /// Create type info for a known type
    pub fn known(schema_id: SchemaId, type_name: String) -> Self {
        let concrete_numeric_type = Self::infer_numeric_runtime_name(&type_name);
        Self {
            schema_id: Some(schema_id),
            type_name: Some(type_name),
            is_definite: true,
            storage_hint: StorageHint::Unknown,
            concrete_numeric_type,
            kind: VariableKind::Value,
            v2_array_element_kind: None,
            v2_struct_layout: None,
        }
    }

    /// Create type info for an unknown/dynamic type
    pub fn unknown() -> Self {
        Self {
            schema_id: None,
            type_name: None,
            is_definite: false,
            storage_hint: StorageHint::Unknown,
            concrete_numeric_type: None,
            kind: VariableKind::Value,
            v2_array_element_kind: None,
            v2_struct_layout: None,
        }
    }

    /// Create type info for a type name that may or may not be registered
    pub fn named(type_name: String) -> Self {
        // Infer storage hint from common type names
        let storage_hint = Self::infer_storage_hint(&type_name);
        let concrete_numeric_type = Self::infer_numeric_runtime_name(&type_name);
        Self {
            schema_id: None,
            type_name: Some(type_name),
            is_definite: false,
            storage_hint,
            concrete_numeric_type,
            kind: VariableKind::Value,
            v2_array_element_kind: None,
            v2_struct_layout: None,
        }
    }

    /// Create type info with explicit storage hint
    pub fn with_storage(type_name: String, storage_hint: StorageHint) -> Self {
        let concrete_numeric_type = Self::infer_numeric_runtime_name(&type_name);
        Self {
            schema_id: None,
            type_name: Some(type_name),
            is_definite: true,
            storage_hint,
            concrete_numeric_type,
            kind: VariableKind::Value,
            v2_array_element_kind: None,
            v2_struct_layout: None,
        }
    }

    /// Create type info for Option<f64> (NaN sentinel optimization)
    pub fn nullable_number() -> Self {
        Self {
            schema_id: None,
            type_name: Some("Option<Number>".to_string()),
            is_definite: true,
            storage_hint: StorageHint::NullableFloat64,
            concrete_numeric_type: Some("f64".to_string()),
            kind: VariableKind::Value,
            v2_array_element_kind: None,
            v2_struct_layout: None,
        }
    }

    /// Create type info for plain f64
    pub fn number() -> Self {
        Self {
            schema_id: None,
            type_name: Some("Number".to_string()),
            is_definite: true,
            storage_hint: StorageHint::Float64,
            concrete_numeric_type: Some("f64".to_string()),
            kind: VariableKind::Value,
            v2_array_element_kind: None,
            v2_struct_layout: None,
        }
    }

    /// Create type info for a RowView variable (typed row from Arrow DataTable).
    pub fn row_view(schema_id: SchemaId, type_name: String) -> Self {
        Self {
            schema_id: Some(schema_id),
            type_name: Some(type_name.clone()),
            is_definite: true,
            storage_hint: StorageHint::Unknown,
            concrete_numeric_type: None,
            kind: VariableKind::RowView {
                element_type: type_name,
            },
            v2_array_element_kind: None,
            v2_struct_layout: None,
        }
    }

    /// Create type info for a DataTable variable with known schema (Table<T>).
    pub fn datatable(schema_id: SchemaId, type_name: String) -> Self {
        Self {
            schema_id: Some(schema_id),
            type_name: Some(type_name.clone()),
            is_definite: true,
            storage_hint: StorageHint::Unknown,
            concrete_numeric_type: None,
            kind: VariableKind::Table {
                element_type: type_name,
            },
            v2_array_element_kind: None,
            v2_struct_layout: None,
        }
    }

    /// Create type info for a Column<T> variable (ColumnRef from Arrow DataTable).
    pub fn column(schema_id: SchemaId, type_name: String, element_type: String) -> Self {
        Self {
            schema_id: Some(schema_id),
            type_name: Some(type_name.clone()),
            is_definite: true,
            storage_hint: StorageHint::Unknown,
            concrete_numeric_type: None,
            kind: VariableKind::Column {
                element_type,
                column_type: type_name,
            },
            v2_array_element_kind: None,
            v2_struct_layout: None,
        }
    }

    /// Create type info for an Indexed table variable — Indexed<T> with known index column.
    pub fn indexed(schema_id: SchemaId, type_name: String, index_column: String) -> Self {
        Self {
            schema_id: Some(schema_id),
            type_name: Some(type_name.clone()),
            is_definite: true,
            storage_hint: StorageHint::Unknown,
            concrete_numeric_type: None,
            kind: VariableKind::Indexed {
                element_type: type_name,
                index_column,
            },
            v2_array_element_kind: None,
            v2_struct_layout: None,
        }
    }

    /// Check if this type is known (has schema ID)
    pub fn is_known(&self) -> bool {
        self.schema_id.is_some()
    }

    /// Check if this type uses NaN sentinel for nullability
    pub fn uses_nan_sentinel(&self) -> bool {
        self.storage_hint == StorageHint::NullableFloat64
    }

    /// Check if this variable is a DataTable (Table<T>)
    pub fn is_datatable(&self) -> bool {
        matches!(self.kind, VariableKind::Table { .. })
    }

    /// Check if this variable is a RowView (Row<T>)
    pub fn is_row_view(&self) -> bool {
        matches!(self.kind, VariableKind::RowView { .. })
    }

    /// Check if this variable is a Column (Column<T>)
    pub fn is_column(&self) -> bool {
        matches!(self.kind, VariableKind::Column { .. })
    }

    /// Check if this variable is an Indexed table (Indexed<T>)
    pub fn is_indexed(&self) -> bool {
        matches!(self.kind, VariableKind::Indexed { .. })
    }

    /// Infer storage hint from type name
    fn infer_storage_hint(type_name: &str) -> StorageHint {
        let trimmed = type_name.trim();

        if let Some(inner) = Self::option_inner_type(trimmed) {
            let inner = inner.trim();
            if let Some(runtime) = BuiltinTypes::canonical_numeric_runtime_name(inner)
                && let Some(hint) = Self::storage_hint_for_runtime_numeric(runtime, true)
            {
                return hint;
            }
            if BuiltinTypes::is_bool_type_name(inner) {
                return StorageHint::Bool;
            }
            if BuiltinTypes::is_string_type_name(inner) {
                return StorageHint::String;
            }
            return StorageHint::Unknown;
        }

        if let Some(runtime) = BuiltinTypes::canonical_numeric_runtime_name(trimmed)
            && let Some(hint) = Self::storage_hint_for_runtime_numeric(runtime, false)
        {
            return hint;
        }
        if BuiltinTypes::is_bool_type_name(trimmed) {
            return StorageHint::Bool;
        }
        if BuiltinTypes::is_string_type_name(trimmed) {
            return StorageHint::String;
        }
        StorageHint::Unknown
    }

    fn option_inner_type(type_name: &str) -> Option<&str> {
        type_name
            .strip_prefix("Option<")
            .and_then(|inner| inner.strip_suffix('>'))
    }

    fn storage_hint_for_runtime_numeric(runtime_name: &str, nullable: bool) -> Option<StorageHint> {
        let base = match runtime_name {
            "f32" | "f64" => StorageHint::Float64,
            "i8" => StorageHint::Int8,
            "u8" => StorageHint::UInt8,
            "i16" => StorageHint::Int16,
            "u16" => StorageHint::UInt16,
            "i32" => StorageHint::Int32,
            "u32" => StorageHint::UInt32,
            "i64" => StorageHint::Int64,
            "u64" => StorageHint::UInt64,
            "isize" => StorageHint::IntSize,
            "usize" => StorageHint::UIntSize,
            _ => return None,
        };
        Some(base.with_nullability(nullable))
    }

    fn infer_numeric_runtime_name(type_name: &str) -> Option<String> {
        let inner = if type_name.starts_with("Option<") && type_name.ends_with('>') {
            &type_name["Option<".len()..type_name.len() - 1]
        } else {
            type_name
        };
        BuiltinTypes::canonical_numeric_runtime_name(inner).map(ToString::to_string)
    }
}

/// Tracks type information for variables during compilation
#[derive(Debug)]
pub struct TypeTracker {
    /// Type schema registry for looking up type definitions
    schema_registry: TypeSchemaRegistry,

    /// Type info for local variables (by slot index)
    local_types: HashMap<u16, VariableTypeInfo>,

    /// Type info for module_binding variables (by slot index)
    binding_types: HashMap<u16, VariableTypeInfo>,

    /// Binding ownership/storage metadata for locals.
    local_binding_semantics: HashMap<u16, BindingSemantics>,

    /// Binding ownership/storage metadata for module bindings.
    binding_semantics: HashMap<u16, BindingSemantics>,

    /// Scoped local type mappings (for scope push/pop)
    local_type_scopes: Vec<HashMap<u16, VariableTypeInfo>>,

    /// Scoped local binding metadata mappings (for scope push/pop).
    local_binding_semantic_scopes: Vec<HashMap<u16, BindingSemantics>>,

    /// Function return types (function name -> type name)
    function_return_types: HashMap<String, String>,
    /// Compile-time object schema contracts: schema id -> field type annotation.
    ///
    /// Used for callable typed-object fields where runtime schema stores only slot layout.
    object_field_contracts: HashMap<SchemaId, HashMap<String, TypeAnnotation>>,

    /// v2: Computed C-compatible struct layouts indexed by SchemaId.
    /// Enables the compiler to look up field offsets at compile time for typed codegen.
    pub v2_layouts: HashMap<SchemaId, StructLayout>,
}

impl TypeTracker {
    /// Create a new type tracker with the given schema registry
    pub fn new(schema_registry: TypeSchemaRegistry) -> Self {
        Self {
            schema_registry,
            local_types: HashMap::new(),
            binding_types: HashMap::new(),
            local_binding_semantics: HashMap::new(),
            binding_semantics: HashMap::new(),
            local_type_scopes: vec![HashMap::new()],
            local_binding_semantic_scopes: vec![HashMap::new()],
            function_return_types: HashMap::new(),
            object_field_contracts: HashMap::new(),
            v2_layouts: HashMap::new(),
        }
    }

    /// Create a new type tracker with an empty registry
    pub fn empty() -> Self {
        Self::new(TypeSchemaRegistry::new())
    }

    /// Create a new type tracker with stdlib types pre-registered
    pub fn with_stdlib() -> Self {
        Self::new(TypeSchemaRegistry::with_stdlib_types())
    }

    /// Get the schema registry
    pub fn schema_registry(&self) -> &TypeSchemaRegistry {
        &self.schema_registry
    }

    /// Get mutable schema registry
    pub fn schema_registry_mut(&mut self) -> &mut TypeSchemaRegistry {
        &mut self.schema_registry
    }

    /// Push a new scope for local types
    pub fn push_scope(&mut self) {
        self.local_type_scopes.push(HashMap::new());
        self.local_binding_semantic_scopes.push(HashMap::new());
    }

    /// Pop a scope, removing local type info for that scope
    pub fn pop_scope(&mut self) {
        if let Some(scope) = self.local_type_scopes.pop() {
            // Remove type info for variables in this scope
            for slot in scope.keys() {
                self.local_types.remove(slot);
            }
        }
        if let Some(scope) = self.local_binding_semantic_scopes.pop() {
            for slot in scope.keys() {
                self.local_binding_semantics.remove(slot);
            }
        }
    }

    /// Set type info for a local variable
    pub fn set_local_type(&mut self, slot: u16, type_info: VariableTypeInfo) {
        // Try to resolve schema ID if we have a type name but no schema ID
        let resolved_info = if type_info.type_name.is_some() && type_info.schema_id.is_none() {
            self.resolve_type_info(type_info)
        } else {
            type_info
        };

        // Track in current scope
        if let Some(scope) = self.local_type_scopes.last_mut() {
            scope.insert(slot, resolved_info.clone());
        }
        self.local_types.insert(slot, resolved_info);
    }

    /// Set type info for a module_binding variable
    pub fn set_binding_type(&mut self, slot: u16, type_info: VariableTypeInfo) {
        let resolved_info = if type_info.type_name.is_some() && type_info.schema_id.is_none() {
            self.resolve_type_info(type_info)
        } else {
            type_info
        };
        self.binding_types.insert(slot, resolved_info);
    }

    /// Set ownership/storage metadata for a local binding.
    pub fn set_local_binding_semantics(&mut self, slot: u16, semantics: BindingSemantics) {
        if let Some(scope) = self.local_binding_semantic_scopes.last_mut() {
            scope.insert(slot, semantics);
        }
        self.local_binding_semantics.insert(slot, semantics);
    }

    /// Set ownership/storage metadata for a module binding.
    pub fn set_binding_semantics(&mut self, slot: u16, semantics: BindingSemantics) {
        self.binding_semantics.insert(slot, semantics);
    }

    /// Update only the storage strategy for a local binding.
    pub fn set_local_binding_storage_class(
        &mut self,
        slot: u16,
        storage_class: BindingStorageClass,
    ) {
        if let Some(existing) = self.local_binding_semantics.get_mut(&slot) {
            existing.storage_class = storage_class;
        }
        for scope in self.local_binding_semantic_scopes.iter_mut().rev() {
            if let Some(existing) = scope.get_mut(&slot) {
                existing.storage_class = storage_class;
                break;
            }
        }
    }

    /// Update only the storage strategy for a module binding.
    pub fn set_binding_storage_class(&mut self, slot: u16, storage_class: BindingStorageClass) {
        if let Some(existing) = self.binding_semantics.get_mut(&slot) {
            existing.storage_class = storage_class;
        }
    }

    /// Get type info for a local variable
    pub fn get_local_type(&self, slot: u16) -> Option<&VariableTypeInfo> {
        self.local_types.get(&slot)
    }

    /// Get type info for a module_binding variable
    pub fn get_binding_type(&self, slot: u16) -> Option<&VariableTypeInfo> {
        self.binding_types.get(&slot)
    }

    /// Get ownership/storage metadata for a local binding.
    pub fn get_local_binding_semantics(&self, slot: u16) -> Option<&BindingSemantics> {
        self.local_binding_semantics.get(&slot)
    }

    /// Get ownership/storage metadata for a module binding.
    pub fn get_binding_semantics(&self, slot: u16) -> Option<&BindingSemantics> {
        self.binding_semantics.get(&slot)
    }

    /// Register a function's return type
    pub fn register_function_return_type(&mut self, func_name: &str, return_type: &str) {
        self.function_return_types
            .insert(func_name.to_string(), return_type.to_string());
    }

    /// Get a function's return type
    pub fn get_function_return_type(&self, func_name: &str) -> Option<&String> {
        self.function_return_types.get(func_name)
    }

    /// Register compile-time field type contracts for an object schema id.
    pub fn register_object_field_contracts(
        &mut self,
        schema_id: SchemaId,
        fields: HashMap<String, TypeAnnotation>,
    ) {
        self.object_field_contracts.insert(schema_id, fields);
    }

    /// Lookup a compile-time field type contract for a schema field.
    pub fn get_object_field_contract(
        &self,
        schema_id: SchemaId,
        field_name: &str,
    ) -> Option<&TypeAnnotation> {
        self.object_field_contracts
            .get(&schema_id)
            .and_then(|fields| fields.get(field_name))
    }

    /// Resolve type name to schema ID
    fn resolve_type_info(&self, mut type_info: VariableTypeInfo) -> VariableTypeInfo {
        if let Some(ref type_name) = type_info.type_name {
            if let Some(schema) = self.schema_registry.get(type_name) {
                type_info.schema_id = Some(schema.id);
                type_info.is_definite = true;
            }
        }
        type_info
    }

    /// Get field offset for typed field access
    ///
    /// Returns (schema_id, field_offset, field_index) if type and field are known
    pub fn get_typed_field_info(
        &self,
        type_name: &str,
        field_name: &str,
    ) -> Option<(SchemaId, usize, u16)> {
        let schema = self.schema_registry.get(type_name)?;
        let field = schema.get_field(field_name)?;
        Some((schema.id, field.offset, field.index))
    }

    /// Get column index for a RowView field access.
    ///
    /// Returns the field index (used as col_id for ColumnAccess operand)
    /// if the variable is a RowView and the field exists in its schema.
    pub fn get_row_view_column_id(
        &self,
        slot: u16,
        is_local: bool,
        field_name: &str,
    ) -> Option<u32> {
        let type_info = if is_local {
            self.get_local_type(slot)?
        } else {
            self.get_binding_type(slot)?
        };
        if !type_info.is_row_view() {
            return None;
        }
        let type_name = type_info.type_name.as_ref()?;
        let schema = self.schema_registry.get(type_name)?;
        let field = schema.get_field(field_name)?;
        Some(field.index as u32)
    }

    /// Check if we can use typed field access for a variable and field
    pub fn can_use_typed_access(&self, slot: u16, is_local: bool, field_name: &str) -> bool {
        let type_info = if is_local {
            self.get_local_type(slot)
        } else {
            self.get_binding_type(slot)
        };

        if let Some(info) = type_info {
            if let Some(ref type_name) = info.type_name {
                return self
                    .schema_registry
                    .field_offset(type_name, field_name)
                    .is_some();
            }
        }
        false
    }

    /// Get storage hint for a local variable
    pub fn get_local_storage_hint(&self, slot: u16) -> StorageHint {
        self.get_local_type(slot)
            .map(|info| info.storage_hint)
            .unwrap_or(StorageHint::Unknown)
    }

    /// Get storage hint for a module_binding variable
    pub fn get_module_binding_storage_hint(&self, slot: u16) -> StorageHint {
        self.get_binding_type(slot)
            .map(|info| info.storage_hint)
            .unwrap_or(StorageHint::Unknown)
    }

    /// Check if a local variable uses NaN sentinel for nullability
    pub fn local_uses_nan_sentinel(&self, slot: u16) -> bool {
        self.get_local_storage_hint(slot) == StorageHint::NullableFloat64
    }

    /// Check if a module_binding variable uses NaN sentinel for nullability
    pub fn module_binding_uses_nan_sentinel(&self, slot: u16) -> bool {
        self.get_module_binding_storage_hint(slot) == StorageHint::NullableFloat64
    }

    /// Clear all local type info (for function entry)
    pub fn clear_locals(&mut self) {
        self.local_types.clear();
        self.local_binding_semantics.clear();
        self.local_type_scopes.clear();
        self.local_type_scopes.push(HashMap::new());
        self.local_binding_semantic_scopes.clear();
        self.local_binding_semantic_scopes.push(HashMap::new());
    }

    /// Register an inline object schema from field names
    ///
    /// Creates a TypeSchema for an object literal with the given fields.
    /// All fields are assumed to be `Any` type (NaN-boxed) since we don't
    /// have full type inference at compile time.
    ///
    /// Returns the SchemaId for use with NewTypedObject opcode.
    ///
    /// # Example
    /// ```ignore
    /// // For: let x = { a: 1, b: "hello" }
    /// let schema_id = tracker.register_inline_object_schema(&["a", "b"]);
    /// // Now emit NewTypedObject with schema_id
    /// ```
    pub fn register_inline_object_schema(&mut self, field_names: &[&str]) -> SchemaId {
        if let Some(existing) = self.schema_registry.type_names().find_map(|name| {
            self.schema_registry.get(name).and_then(|schema| {
                if schema.fields.len() != field_names.len() {
                    return None;
                }
                let same_order = schema
                    .fields
                    .iter()
                    .map(|f| f.name.as_str())
                    .eq(field_names.iter().copied());
                if same_order { Some(schema.id) } else { None }
            })
        }) {
            return existing;
        }

        // Generate a unique name for this inline object type
        let id = INLINE_OBJECT_COUNTER.fetch_add(1, Ordering::SeqCst);
        let type_name = format!("__inline_obj_{}", id);

        // Create field definitions - all fields are Any (NaN-boxed)
        let fields: Vec<(String, FieldType)> = field_names
            .iter()
            .map(|name| (name.to_string(), FieldType::Any))
            .collect();

        // Create and register the schema
        let schema = TypeSchema::new(&type_name, fields);
        let schema_id = schema.id;
        self.schema_registry.register(schema);

        schema_id
    }

    /// Register an inline object schema with typed fields
    ///
    /// Like `register_inline_object_schema` but allows specifying field types
    /// for better JIT optimization. Deduplicates by matching both field names
    /// and types.
    pub fn register_inline_object_schema_typed(
        &mut self,
        fields: &[(&str, FieldType)],
    ) -> SchemaId {
        if let Some(existing) = self.schema_registry.type_names().find_map(|name| {
            self.schema_registry.get(name).and_then(|schema| {
                if schema.fields.len() != fields.len() {
                    return None;
                }
                let same = schema
                    .fields
                    .iter()
                    .zip(fields.iter())
                    .all(|(f, (n, t))| f.name == *n && f.field_type == *t);
                if same { Some(schema.id) } else { None }
            })
        }) {
            return existing;
        }

        let id = INLINE_OBJECT_COUNTER.fetch_add(1, Ordering::SeqCst);
        let type_name = format!("__inline_obj_{}", id);
        let field_defs: Vec<(String, FieldType)> = fields
            .iter()
            .map(|(name, ft)| (name.to_string(), ft.clone()))
            .collect();
        let schema = TypeSchema::new(&type_name, field_defs);
        let schema_id = schema.id;
        self.schema_registry.register(schema);
        schema_id
    }

    /// Register a named struct schema (e.g. `Point { x, y }`)
    ///
    /// Unlike `register_inline_object_schema` which auto-generates names,
    /// this uses the actual struct type name so `.type()` can resolve it.
    pub fn register_named_object_schema(
        &mut self,
        type_name: &str,
        fields: &[(&str, FieldType)],
    ) -> SchemaId {
        let field_defs: Vec<(String, FieldType)> = fields
            .iter()
            .map(|(name, ft)| (name.to_string(), ft.clone()))
            .collect();

        let schema = TypeSchema::new(type_name, field_defs);
        let schema_id = schema.id;
        self.schema_registry.register(schema);

        schema_id
    }

    /// Register an inline object schema with typed fields
    ///
    /// Like `register_inline_object_schema` but allows specifying field types
    /// for better JIT optimization.
    pub fn register_typed_object_schema(
        &mut self,
        field_defs: Vec<(String, FieldType)>,
    ) -> SchemaId {
        let id = INLINE_OBJECT_COUNTER.fetch_add(1, Ordering::SeqCst);
        let type_name = format!("__inline_obj_{}", id);

        let schema = TypeSchema::new(&type_name, field_defs);
        let schema_id = schema.id;
        self.schema_registry.register(schema);

        schema_id
    }

    // --- v2 helpers ---

    /// Register a v2 StructLayout for the given schema ID.
    pub fn register_v2_layout(&mut self, schema_id: SchemaId, layout: StructLayout) {
        self.v2_layouts.insert(schema_id, layout);
    }

    /// Look up a v2 StructLayout by schema ID.
    pub fn get_v2_layout(&self, schema_id: SchemaId) -> Option<&StructLayout> {
        self.v2_layouts.get(&schema_id)
    }

    /// Check if a local slot is a typed array and return its element kind.
    pub fn is_typed_array(&self, slot: u16) -> Option<FieldKind> {
        self.local_types.get(&slot)?.v2_array_element_kind
    }

    /// Check if a local slot has a v2 struct layout and return its schema ID.
    pub fn is_typed_struct(&self, slot: u16) -> Option<SchemaId> {
        self.local_types.get(&slot)?.v2_struct_layout
    }
}

impl Default for TypeTracker {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::TypeSchemaBuilder;

    #[test]
    fn test_basic_type_tracking() {
        let mut registry = TypeSchemaRegistry::new();

        TypeSchemaBuilder::new("Point")
            .f64_field("x")
            .f64_field("y")
            .register(&mut registry);

        let mut tracker = TypeTracker::new(registry);

        // Set type for local slot 0
        tracker.set_local_type(0, VariableTypeInfo::named("Point".to_string()));

        // Check that we can use typed access
        assert!(tracker.can_use_typed_access(0, true, "x"));
        assert!(tracker.can_use_typed_access(0, true, "y"));
        assert!(!tracker.can_use_typed_access(0, true, "z")); // Unknown field
    }

    #[test]
    fn test_scope_tracking() {
        let mut tracker = TypeTracker::empty();

        // Declare in outer scope
        tracker.set_local_type(0, VariableTypeInfo::named("Outer".to_string()));

        // Push inner scope
        tracker.push_scope();
        tracker.set_local_type(1, VariableTypeInfo::named("Inner".to_string()));

        assert!(tracker.get_local_type(0).is_some());
        assert!(tracker.get_local_type(1).is_some());

        // Pop inner scope
        tracker.pop_scope();

        // Outer still exists, inner removed
        assert!(tracker.get_local_type(0).is_some());
        assert!(tracker.get_local_type(1).is_none());
    }

    #[test]
    fn test_binding_semantics_scope_tracking() {
        let mut tracker = TypeTracker::empty();

        tracker.set_local_binding_semantics(
            0,
            BindingSemantics::deferred(BindingOwnershipClass::OwnedImmutable),
        );
        tracker.set_binding_semantics(
            5,
            BindingSemantics::deferred(BindingOwnershipClass::Flexible),
        );

        tracker.push_scope();
        tracker.set_local_binding_semantics(
            1,
            BindingSemantics::deferred(BindingOwnershipClass::OwnedMutable),
        );

        assert_eq!(
            tracker
                .get_local_binding_semantics(0)
                .map(|s| s.ownership_class),
            Some(BindingOwnershipClass::OwnedImmutable)
        );
        assert_eq!(
            tracker
                .get_local_binding_semantics(1)
                .map(|s| s.ownership_class),
            Some(BindingOwnershipClass::OwnedMutable)
        );
        assert_eq!(
            tracker.get_binding_semantics(5).map(|s| s.ownership_class),
            Some(BindingOwnershipClass::Flexible)
        );

        tracker.pop_scope();

        assert!(tracker.get_local_binding_semantics(1).is_none());
        assert!(tracker.get_local_binding_semantics(0).is_some());
        assert!(tracker.get_binding_semantics(5).is_some());
    }

    #[test]
    fn test_binding_storage_class_updates() {
        let mut tracker = TypeTracker::empty();
        tracker.set_local_binding_semantics(
            0,
            BindingSemantics::deferred(BindingOwnershipClass::OwnedMutable),
        );
        tracker.set_binding_semantics(
            4,
            BindingSemantics::deferred(BindingOwnershipClass::Flexible),
        );

        tracker.set_local_binding_storage_class(0, BindingStorageClass::Reference);
        tracker.set_binding_storage_class(4, BindingStorageClass::SharedCow);

        assert_eq!(
            tracker
                .get_local_binding_semantics(0)
                .map(|s| s.storage_class),
            Some(BindingStorageClass::Reference)
        );
        assert_eq!(
            tracker.get_binding_semantics(4).map(|s| s.storage_class),
            Some(BindingStorageClass::SharedCow)
        );

        tracker.clear_locals();
        assert!(tracker.get_local_binding_semantics(0).is_none());
        assert!(tracker.get_binding_semantics(4).is_some());
    }

    #[test]
    fn test_function_return_types() {
        let mut tracker = TypeTracker::empty();

        tracker.register_function_return_type("get_point", "Point");

        assert_eq!(
            tracker.get_function_return_type("get_point"),
            Some(&"Point".to_string())
        );
        assert!(tracker.get_function_return_type("unknown").is_none());
    }

    #[test]
    fn test_typed_field_info() {
        let mut registry = TypeSchemaRegistry::new();

        TypeSchemaBuilder::new("Vector3")
            .f64_field("x")
            .f64_field("y")
            .f64_field("z")
            .register(&mut registry);

        let tracker = TypeTracker::new(registry);

        let info = tracker.get_typed_field_info("Vector3", "y");
        assert!(info.is_some());
        let (schema_id, offset, index) = info.unwrap();
        assert!(schema_id > 0);
        assert_eq!(offset, 8); // Second field, 8 bytes offset
        assert_eq!(index, 1);
    }

    #[test]
    fn test_unknown_type() {
        let tracker = TypeTracker::empty();

        // Unknown type should not allow typed access
        assert!(!tracker.can_use_typed_access(0, true, "field"));
    }

    #[test]
    fn test_binding_type_tracking() {
        let mut registry = TypeSchemaRegistry::new();

        TypeSchemaBuilder::new("Config")
            .f64_field("threshold")
            .string_field("name")
            .register(&mut registry);

        let mut tracker = TypeTracker::new(registry);

        // Set type for module_binding slot 5
        tracker.set_binding_type(5, VariableTypeInfo::named("Config".to_string()));

        assert!(tracker.can_use_typed_access(5, false, "threshold"));
        assert!(tracker.can_use_typed_access(5, false, "name"));
        assert!(!tracker.can_use_typed_access(5, false, "unknown"));
    }

    #[test]
    fn test_storage_hint_inference() {
        // Primitive types
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("Number"),
            StorageHint::Float64
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("Integer"),
            StorageHint::Int64
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("Bool"),
            StorageHint::Bool
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("String"),
            StorageHint::String
        );

        // Nullable types
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("Option<Number>"),
            StorageHint::NullableFloat64
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("Option<Integer>"),
            StorageHint::NullableInt64
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("Option<byte>"),
            StorageHint::NullableUInt8
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("Option<char>"),
            StorageHint::NullableInt8
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("Option<u32>"),
            StorageHint::NullableUInt32
        );

        // Unknown types
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("SomeCustomType"),
            StorageHint::Unknown
        );
    }

    #[test]
    fn test_width_integer_storage_hint_inference() {
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("i8"),
            StorageHint::Int8
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("byte"),
            StorageHint::UInt8
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("char"),
            StorageHint::Int8
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("u16"),
            StorageHint::UInt16
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("i32"),
            StorageHint::Int32
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("u64"),
            StorageHint::UInt64
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("isize"),
            StorageHint::IntSize
        );
        assert_eq!(
            VariableTypeInfo::infer_storage_hint("usize"),
            StorageHint::UIntSize
        );
    }

    #[test]
    fn test_concrete_numeric_type_inference() {
        assert_eq!(
            VariableTypeInfo::infer_numeric_runtime_name("int"),
            Some("i64".to_string())
        );
        assert_eq!(
            VariableTypeInfo::infer_numeric_runtime_name("i16"),
            Some("i16".to_string())
        );
        assert_eq!(
            VariableTypeInfo::infer_numeric_runtime_name("byte"),
            Some("u8".to_string())
        );
        assert_eq!(
            VariableTypeInfo::infer_numeric_runtime_name("Option<f32>"),
            Some("f32".to_string())
        );
        assert_eq!(
            VariableTypeInfo::infer_numeric_runtime_name("SomeCustomType"),
            None
        );
    }

    #[test]
    fn test_storage_hint_from_storage_type() {
        assert_eq!(
            StorageHint::from_storage_type(&StorageType::Float64),
            StorageHint::Float64
        );
        assert_eq!(
            StorageHint::from_storage_type(&StorageType::NullableFloat64),
            StorageHint::NullableFloat64
        );
        assert_eq!(
            StorageHint::from_storage_type(&StorageType::Dynamic),
            StorageHint::Unknown
        );
    }

    #[test]
    fn test_nullable_number_type() {
        let info = VariableTypeInfo::nullable_number();
        assert!(info.uses_nan_sentinel());
        assert_eq!(info.storage_hint, StorageHint::NullableFloat64);
    }

    #[test]
    fn test_row_view_column_id_resolution() {
        let mut registry = TypeSchemaRegistry::new();

        TypeSchemaBuilder::new("Candle")
            .f64_field("open")
            .f64_field("high")
            .f64_field("low")
            .f64_field("close")
            .i64_field("volume")
            .register(&mut registry);

        let mut tracker = TypeTracker::new(registry);

        // Get schema ID for Candle
        let schema = tracker.schema_registry().get("Candle").unwrap();
        let schema_id = schema.id;

        // Set local slot 0 as a RowView<Candle>
        tracker.set_local_type(
            0,
            VariableTypeInfo::row_view(schema_id, "Candle".to_string()),
        );

        // Should resolve known fields
        assert_eq!(tracker.get_row_view_column_id(0, true, "open"), Some(0));
        assert_eq!(tracker.get_row_view_column_id(0, true, "high"), Some(1));
        assert_eq!(tracker.get_row_view_column_id(0, true, "close"), Some(3));
        assert_eq!(tracker.get_row_view_column_id(0, true, "volume"), Some(4));

        // Should return None for unknown fields
        assert_eq!(tracker.get_row_view_column_id(0, true, "nonexistent"), None);

        // Non-row-view variable should return None
        tracker.set_local_type(1, VariableTypeInfo::named("Candle".to_string()));
        assert_eq!(tracker.get_row_view_column_id(1, true, "open"), None);
    }

    #[test]
    fn test_tracker_storage_hints() {
        let mut tracker = TypeTracker::empty();

        // Set local with nullable type
        tracker.set_local_type(0, VariableTypeInfo::nullable_number());
        assert!(tracker.local_uses_nan_sentinel(0));

        // Set local with regular number
        tracker.set_local_type(1, VariableTypeInfo::number());
        assert!(!tracker.local_uses_nan_sentinel(1));

        // Unknown slot
        assert!(!tracker.local_uses_nan_sentinel(99));
    }

    #[test]
    fn test_datatable_type_info() {
        let mut registry = TypeSchemaRegistry::new();

        TypeSchemaBuilder::new("Trade")
            .f64_field("price")
            .i64_field("volume")
            .string_field("symbol")
            .register(&mut registry);

        let mut tracker = TypeTracker::new(registry);

        let schema = tracker.schema_registry().get("Trade").unwrap();
        let schema_id = schema.id;

        // Create a datatable type info
        tracker.set_local_type(
            0,
            VariableTypeInfo::datatable(schema_id, "Trade".to_string()),
        );

        let info = tracker.get_local_type(0).unwrap();
        assert!(info.is_datatable());
        assert!(!info.is_row_view());
        assert_eq!(info.schema_id, Some(schema_id));
        assert_eq!(info.type_name.as_deref(), Some("Trade"));

        // RowView should not be a datatable
        tracker.set_local_type(
            1,
            VariableTypeInfo::row_view(schema_id, "Trade".to_string()),
        );
        let info = tracker.get_local_type(1).unwrap();
        assert!(!info.is_datatable());
        assert!(info.is_row_view());
    }

    #[test]
    fn test_v2_struct_layout_registration() {
        use shape_value::v2::struct_layout::{FieldKind, StructLayout};

        let mut tracker = TypeTracker::empty();

        let layout = StructLayout::new(&[
            ("x", FieldKind::F64),
            ("y", FieldKind::F64),
        ]);
        assert_eq!(layout.total_size(), 24);

        // Use a fake schema ID
        let schema_id: SchemaId = 42;
        tracker.register_v2_layout(schema_id, layout);

        let retrieved = tracker.get_v2_layout(schema_id);
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.field_count(), 2);
        assert_eq!(retrieved.field_offset(0), 8);
        assert_eq!(retrieved.field_offset(1), 16);
        assert_eq!(retrieved.total_size(), 24);

        // Non-existent schema ID returns None
        assert!(tracker.get_v2_layout(999).is_none());
    }

    #[test]
    fn test_v2_typed_array_element_kind() {
        use shape_value::v2::struct_layout::FieldKind;

        let mut tracker = TypeTracker::empty();

        // Set up a local as a typed array of F64
        let mut info = VariableTypeInfo::named("Array<number>".to_string());
        info.v2_array_element_kind = Some(FieldKind::F64);
        tracker.set_local_type(0, info);

        assert_eq!(tracker.is_typed_array(0), Some(FieldKind::F64));
        assert_eq!(tracker.is_typed_array(1), None); // unset slot

        // Set up a local as a typed array of I32
        let mut info2 = VariableTypeInfo::named("Array<i32>".to_string());
        info2.v2_array_element_kind = Some(FieldKind::I32);
        tracker.set_local_type(1, info2);

        assert_eq!(tracker.is_typed_array(1), Some(FieldKind::I32));
    }

    #[test]
    fn test_v2_typed_struct_on_variable() {
        use shape_value::v2::struct_layout::{FieldKind, StructLayout};

        let mut tracker = TypeTracker::empty();

        let layout = StructLayout::new(&[
            ("name", FieldKind::Ptr),
            ("age", FieldKind::I32),
            ("score", FieldKind::F64),
        ]);
        let schema_id: SchemaId = 100;
        tracker.register_v2_layout(schema_id, layout);

        // Set up a local with v2_struct_layout
        let mut info = VariableTypeInfo::named("Person".to_string());
        info.v2_struct_layout = Some(schema_id);
        tracker.set_local_type(0, info);

        // Verify is_typed_struct returns the schema ID
        assert_eq!(tracker.is_typed_struct(0), Some(schema_id));
        assert_eq!(tracker.is_typed_struct(1), None);

        // Verify we can look up the layout from the schema ID
        let layout = tracker.get_v2_layout(schema_id).unwrap();
        assert_eq!(layout.field_count(), 3);
        assert_eq!(layout.field_kind(0), FieldKind::Ptr);
        assert_eq!(layout.field_kind(1), FieldKind::I32);
        assert_eq!(layout.field_kind(2), FieldKind::F64);
        assert_eq!(layout.heap_field_mask, 0b001); // only field 0 is Ptr
    }

    #[test]
    fn test_v2_fields_default_none() {
        // All constructors should default v2 fields to None
        let info = VariableTypeInfo::unknown();
        assert!(info.v2_array_element_kind.is_none());
        assert!(info.v2_struct_layout.is_none());

        let info = VariableTypeInfo::number();
        assert!(info.v2_array_element_kind.is_none());
        assert!(info.v2_struct_layout.is_none());

        let info = VariableTypeInfo::named("Foo".to_string());
        assert!(info.v2_array_element_kind.is_none());
        assert!(info.v2_struct_layout.is_none());

        let info = VariableTypeInfo::known(1, "Bar".to_string());
        assert!(info.v2_array_element_kind.is_none());
        assert!(info.v2_struct_layout.is_none());
    }
}
