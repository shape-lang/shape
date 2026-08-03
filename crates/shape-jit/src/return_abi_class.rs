//! Return ABI classes for value-producing FFI entry points (ADR-020 / #239 §4.1).
//!
//! Cranelift return types are per-signature, so the monomorph set is driven by
//! Cranelift ABI classes rather than by all 27 `NativeKind` variants. An entry
//! point that used to be `-> u64` becomes one function per class, and the emit
//! site picks the class from the destination slot's PROVEN kind.
//!
//! The classification is anchored on `mir_compiler::types::cranelift_type_for_slot`
//! — the same table that decides what Cranelift type the destination variable is
//! declared at. Anchoring anywhere else would let a monomorph return a type the
//! destination variable cannot hold.
//!
//! ## Why `Scalar` and `Pointer` are separate (§4.2)
//!
//! They lower identically: `*mut HeapHeader` and `i64` are both `types::I64`, so
//! the split costs nothing at the ABI. At the Rust level they are different
//! types, and that is what lets rustc distinguish "this return transfers an owned
//! share" from "this return is a number". Collapsing them into one `i64` would
//! erase the only remaining carrier of that distinction:
//!
//! > **Invariant O1.** An FFI call's heap return transfers exactly ONE owned
//! > share to the caller. The emit site releases the destination's old value and
//! > stores the new one without retaining.
//! >
//! > **Invariant O2.** A converted entry point returns `*mut HeapHeader` IFF it
//! > transfers a share; it returns `i64` / `f64` iff it transfers nothing. There
//! > is no third case — a borrowed heap pointer is not a legal return, because
//! > the callee cannot bound the borrow's lifetime across the FFI edge.

use shape_value::NativeKind;

/// The Cranelift ABI class of a value-producing FFI return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReturnAbiClass {
    /// `i64` / `types::I64`. Integers of every width (narrowed in-slot by
    /// `ensure_kind` at the destination), `Bool`, `Char`, `Null`.
    Scalar,
    /// `*mut HeapHeader` / `types::I64`. Transfers one owned share (O1/O2).
    Pointer,
    /// `f64` / `types::F64`. The value travels in an FP register; it never
    /// becomes a bit pattern at the Cranelift boundary, which is how
    /// `box_number` dies on this path.
    Float,
    /// No results. ADR-020 §3.3 — the callee produced no value at all, and the
    /// destination slot carries unit.
    Void,
}

impl ReturnAbiClass {
    /// Short name for diagnostics — matches the FFI symbol suffix.
    pub(crate) fn suffix(self) -> &'static str {
        match self {
            ReturnAbiClass::Scalar => "i64",
            ReturnAbiClass::Pointer => "ptr",
            ReturnAbiClass::Float => "f64",
            ReturnAbiClass::Void => "void",
        }
    }
}

/// The ABI class a value of this `NativeKind` is returned in.
///
/// Total over `NativeKind` by construction — there is no "unclassified" answer
/// and no default. A caller that has no kind at all must surface BEFORE reaching
/// here; it must not pass a stand-in (#236 / R-G7).
pub(crate) fn return_abi_class(kind: NativeKind) -> ReturnAbiClass {
    match kind {
        // Only `Float64` maps to `types::F64` in `cranelift_type_for_slot`.
        // `NullableFloat64` is I64 there (its presence pair is #229's work and
        // has no producer at HEAD), so it classifies Scalar and stays
        // consistent with the destination variable's declared type.
        NativeKind::Float64 => ReturnAbiClass::Float,

        // Heap carriers — one owned share transfers (O2).
        NativeKind::Ptr(_)
        | NativeKind::String
        | NativeKind::StringV2
        | NativeKind::DecimalV2 => ReturnAbiClass::Pointer,

        // Everything else is an inline scalar: the signed/unsigned width family
        // and their nullable forms, `Bool`, `Char`, `Float32`, and `Null` (the
        // merged absence sentinel — a value with zero bits, not the absence of
        // a value; absence-of-a-value is `Void`, selected from `unit_slots`).
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize
        | NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize
        | NativeKind::NullableFloat64
        | NativeKind::Float32
        | NativeKind::Char
        | NativeKind::Bool
        | NativeKind::Null => ReturnAbiClass::Scalar,
    }
}

/// True when a value actually returned under `actual` may be handed back
/// through a monomorph declared for `expected`.
///
/// This is the §10.2 kind-agreement assertion at the FFI half of the boundary:
/// the emit site chose the monomorph from the destination's proven kind, and the
/// callee produced a value of its own kind. Agreement is required at the CLASS
/// level, not the kind level — an `Int32`-returning callee legitimately feeds an
/// `Int64` destination through `ensure_kind`'s in-slot narrowing, and both are
/// `Scalar`. Disagreement across classes is a producing-site bug: a scalar
/// monomorph writing into a `Ptr(_)`-stamped slot is the `ClosurePlaceholder`
/// defect (§6.1) in general form.
pub(crate) fn classes_agree(expected: ReturnAbiClass, actual: NativeKind) -> bool {
    return_abi_class(actual) == expected
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::heap_value::HeapKind;

    #[test]
    fn float64_is_the_only_float_class() {
        assert_eq!(return_abi_class(NativeKind::Float64), ReturnAbiClass::Float);
        // The live `cranelift_type_for_slot` table maps NullableFloat64 to I64
        // via its catch-all, so classifying it Float here would return an F64
        // into an I64-declared destination variable.
        assert_eq!(
            return_abi_class(NativeKind::NullableFloat64),
            ReturnAbiClass::Scalar
        );
        assert_eq!(return_abi_class(NativeKind::Float32), ReturnAbiClass::Scalar);
    }

    #[test]
    fn heap_carriers_are_pointer_class() {
        for k in [
            NativeKind::String,
            NativeKind::StringV2,
            NativeKind::DecimalV2,
            NativeKind::Ptr(HeapKind::Closure),
            NativeKind::Ptr(HeapKind::TypedArray),
            NativeKind::Ptr(HeapKind::TypedObject),
        ] {
            assert_eq!(return_abi_class(k), ReturnAbiClass::Pointer, "{k:?}");
        }
    }

    #[test]
    fn narrow_ints_share_the_scalar_class_with_int64() {
        // The in-slot narrowing happens at `ensure_kind`, not at the ABI, so a
        // narrow-returning callee and a wide destination agree.
        assert!(classes_agree(ReturnAbiClass::Scalar, NativeKind::Int32));
        assert!(classes_agree(ReturnAbiClass::Scalar, NativeKind::Bool));
        assert!(classes_agree(ReturnAbiClass::Scalar, NativeKind::Char));
        assert!(classes_agree(ReturnAbiClass::Scalar, NativeKind::Null));
    }

    #[test]
    fn a_heap_return_into_a_scalar_monomorph_disagrees() {
        // This is the assertion that would have caught the ClosurePlaceholder
        // defect at the emit site.
        assert!(!classes_agree(
            ReturnAbiClass::Scalar,
            NativeKind::Ptr(HeapKind::Closure)
        ));
        assert!(!classes_agree(ReturnAbiClass::Pointer, NativeKind::Int64));
        assert!(!classes_agree(ReturnAbiClass::Float, NativeKind::Int64));
        assert!(!classes_agree(ReturnAbiClass::Scalar, NativeKind::Float64));
    }

    #[test]
    fn every_class_has_a_distinct_symbol_suffix() {
        let all = [
            ReturnAbiClass::Scalar,
            ReturnAbiClass::Pointer,
            ReturnAbiClass::Float,
            ReturnAbiClass::Void,
        ];
        let mut seen: Vec<&str> = all.iter().map(|c| c.suffix()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), all.len());
    }
}
