//! `NativeKind`: the single discriminator for typed values at every ABI exit.
//!
//! Used by:
//! - `shape-vm` compile-time proof: `prove_native_kind() -> Result<NativeKind, ProofGap>`
//! - Marshal layer (`shape-runtime::typed_module_exports`): `(u64 bits, NativeKind kind)` paired
//! - Wire/snapshot serialization: `slot_to_wire(bits, kind, ctx)`
//! - JIT FFI boundary
//!
//! Previously named `SlotKind`; renamed and moved out of `shape-vm/type_tracking.rs`
//! into the foundational `shape-value` crate during the strict-typing Phase 2b
//! marshal-layer landing. The single-discriminator rule prevents the two-parallel-
//! discriminator drift trap (see `docs/defections.md` 2026-05-06 — Phase 2b).
//!
//! `NativeKind::Dynamic` and `NativeKind::Unknown` are both deleted — the bulldozer
//! removed them per the strict-typed plan. Every slot has a proven kind at compile
//! time or it's a compile error. There is no fallback variant.

use crate::heap_value::HeapKind;
use serde::{Deserialize, Serialize};

/// Storage discriminator for a single 8-byte typed slot.
///
/// Each variant identifies which native type the slot's `u64` raw bits
/// represent, including width and nullability for integers and float.
/// Boolean has no width variant. `String` is special-cased as the most
/// common heap shape (an `Arc<String>` raw pointer); all other heap-
/// allocated shapes use `Ptr(HeapKind)` carrying the surviving
/// `HeapValue` discriminant. The kind tells the marshal/wire/snapshot
/// layer which `HeapValue` arm the bits decode to without probing the
/// bits themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NativeKind {
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
    /// String reference (Arc<String> raw pointer)
    String,
    /// Heap pointer (`Arc<HeapValue>` raw pointer) whose `HeapValue`
    /// discriminant is `kind`. The marshal/wire/snapshot layer dispatches
    /// on `kind` to project the slot to its typed shape — it does not
    /// probe the heap object's self-reported discriminant in production
    /// (`(*hv).kind() == kind` is a debug-only sanity check).
    ///
    /// Watchlist (`docs/defections.md` 2026-05-06 — HeapKind trim +
    /// Ptr extension): do NOT add parametric `NativeKind::Result(..)`,
    /// `NativeKind::Option(..)`, or `NativeKind::JsonValue` variants
    /// when stdlib mass migration hits those returns. The strict-typed
    /// answer is `HeapKind::TypedObject` plus a per-instantiation
    /// schema_id from the function's registered `ConcreteType`. Adding
    /// parametric NativeKind variants re-creates heterogeneous-by-default
    /// sum types at the discriminator level — the same defection
    /// pattern as the rejected `enum SlotValue { Int, Float, Bool, Heap }`.
    ///
    // ADR-005 names the general principle this watchlist applies at
    // the proof layer: HeapValue is the single discriminator for
    // heap-resident values; layers above take Arc<HeapValue> and dispatch
    // on HeapValue::kind(). See docs/adr/005-typed-slot-construction.md.
    Ptr(HeapKind),
    // NativeKind::Dynamic and NativeKind::Unknown deleted by the strict-typing
    // bulldozer (commit 128cb8a). There is no dynamic-typed slot. Every slot
    // has a proven NativeKind at compile time or it's a compile error.
    // Default impl also deleted — call sites must commit to a concrete
    // kind, not rely on "Unknown means I haven't decided yet".
}

impl NativeKind {
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

    /// Whether values of this kind carry a refcounted heap pointer.
    ///
    /// Post-strict-typing (ADR-006 §2.7.5 / §2.7.6 / Q8), the kind IS the
    /// discriminator that decides refcount semantics — there is no
    /// tag-bit probing. Heap kinds are `String` (Arc<String> raw pointer)
    /// and `Ptr(HeapKind::*)` (Arc<HeapValue> raw pointer). All
    /// numeric / bool kinds — including their nullable variants — are
    /// raw scalars and do NOT carry a refcount, regardless of Cranelift
    /// storage width (an `Int64` slot is a raw `i64`, not a NaN-boxed
    /// ValueWord; the deleted ValueWord ABI is what made the W-series
    /// `is_native_slot` predicate exclude Int64).
    ///
    /// Used by `shape-jit/src/mir_compiler/ownership.rs` to gate
    /// `jit_arc_retain` / `jit_arc_release` emission. The kind-blind
    /// fall-through ("if kind isn't proven, assume heap and retain")
    /// the prior W-series MIR emitter took is forbidden under §2.7.7
    /// #9 — when kind isn't proven, surface-and-stop is the principled
    /// response, not a Bool-default-like silent retain.
    #[inline]
    pub fn is_refcounted(self) -> bool {
        matches!(self, Self::String | Self::Ptr(_))
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
