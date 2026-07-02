//! Comparison operations for the VM executor (ADR-006 §2.7.7 / Q9 — kinded stack).
//!
//! Handles: Gt, Lt, Gte, Lte, Eq, Neq (typed variants per primitive
//! kind: Int/Number/Decimal/String).
//!
//! Wave 6.5 substep-2 (Cluster A): every push/pop now threads through the
//! kinded API (`push_kinded(bits, kind)` / `pop_kinded()`). Result kind for
//! every comparison opcode is `NativeKind::Bool` (per playbook §2 — the
//! comparison row of the kind-sourcing table). Operand-side dispatch on
//! kind via the kinded API + `as_heap_value()` for heap-backed kinds; no
//! `stack_top_both_*` fast paths (the dual-path probes were deleted in
//! substep-1 and read-as-u64 cannot detect kind without the parallel
//! kinds track, which is queried via `pop_kinded` here).

#![allow(clippy::approx_constant)] // arbitrary test floats; not math constants
use crate::{
    bytecode::{Instruction, OpCode},
    executor::VirtualMachine,
    executor::vm_impl::stack::drop_with_kind,
};
use shape_value::{
    NativeKind, VMError, ValueSlot,
    heap_value::{HeapKind, HeapValue, TypedObjectStorage},
};
use std::cmp::Ordering;
use std::sync::Arc;

use crate::constants::EXACT_F64_INT_LIMIT;

impl VirtualMachine {
    #[inline(always)]
    fn i128_to_lossless_f64(v: i128) -> Option<f64> {
        if (-EXACT_F64_INT_LIMIT..=EXACT_F64_INT_LIMIT).contains(&v) {
            Some(v as f64)
        } else {
            None
        }
    }

    /// Compare two raw `(bits, kind)` pairs as numeric values without lossy
    /// integer→float coercion. Returns `None` for non-numeric kinds or
    /// numerically-incomparable pairs (e.g. NaN).
    #[inline(always)]
    fn nb_compare_numeric_kinded(
        a_bits: u64,
        a_kind: NativeKind,
        b_bits: u64,
        b_kind: NativeKind,
    ) -> Option<Ordering> {
        // Domain coercion helpers — pull a numeric value out of (bits, kind)
        // without consuming the share.
        let a_int = numeric_as_i128(a_bits, a_kind);
        let b_int = numeric_as_i128(b_bits, b_kind);
        if let (Some(ai), Some(bi)) = (a_int, b_int) {
            return Some(ai.cmp(&bi));
        }

        let a_dec = numeric_as_decimal_ref(a_bits, a_kind);
        let b_dec = numeric_as_decimal_ref(b_bits, b_kind);
        match (a_dec, b_dec) {
            (Some(ad), Some(bd)) => return Some(ad.cmp(bd)),
            (Some(ad), None) => {
                if let Some(bi) = b_int {
                    let b_dec = rust_decimal::Decimal::from_i128_with_scale(bi, 0);
                    return Some(ad.cmp(&b_dec));
                }
                if let Some(bf) = numeric_as_f64(b_bits, b_kind) {
                    let b_dec = rust_decimal::Decimal::from_f64_retain(bf)?;
                    return Some(ad.cmp(&b_dec));
                }
            }
            (None, Some(bd)) => {
                if let Some(ai) = a_int {
                    let a_dec = rust_decimal::Decimal::from_i128_with_scale(ai, 0);
                    return Some(a_dec.cmp(bd));
                }
                if let Some(af) = numeric_as_f64(a_bits, a_kind) {
                    let a_dec = rust_decimal::Decimal::from_f64_retain(af)?;
                    return Some(a_dec.cmp(bd));
                }
            }
            _ => {}
        }

        let a_f = numeric_as_f64(a_bits, a_kind);
        let b_f = numeric_as_f64(b_bits, b_kind);
        if let (Some(af), Some(bf)) = (a_f, b_f) {
            return af.partial_cmp(&bf);
        }
        if let (Some(ai), Some(bf)) = (a_int, b_f) {
            let af = Self::i128_to_lossless_f64(ai)?;
            return af.partial_cmp(&bf);
        }
        if let (Some(af), Some(bi)) = (a_f, b_int) {
            let bf = Self::i128_to_lossless_f64(bi)?;
            return af.partial_cmp(&bf);
        }

        None
    }

    /// Execute typed comparison opcodes (compiler-guaranteed types, zero dispatch)
    #[inline(always)]
    pub(in crate::executor) fn exec_typed_comparison(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(ref mut metrics) = self.metrics {
            if instruction.opcode.is_trusted() {
                metrics.record_trusted_op();
            } else {
                metrics.record_guarded_op();
            }
        }
        use OpCode::*;
        match instruction.opcode {
            // ===== Int family — typed pop, kinded bool push =====
            //
            // Wave 6.5: Int comparisons read native i64 bits via `pop_kinded`
            // and unconditionally push `NativeKind::Bool`. The pre-Wave-6
            // dual-path `is_tagged()` call is gone (deleted in substep-1;
            // would always have returned false on native bits).
            // R5c-2-β-γ checkpoint (b) u64-carrier: ordering comparisons are
            // signedness-DEPENDENT. `u64::MAX` (`0xFFFF…`) must compare
            // GREATER than `2`, not less. The pre-checkpoint-(b) handlers
            // discarded the operand kind and always reinterpreted bits as
            // signed `i64`, so `(u64::MAX) > (2)` evaluated `(-1) > 2 ==
            // false`. The slot KIND — producer-stamped per ADR-006 §2.7.5 —
            // is the discriminator: a `NativeKind::UInt64` operand decodes
            // its bits as `u64` and uses an unsigned comparison. This
            // reuses the existing `*Int` comparison opcode machinery
            // (no new opcode family) — the operand kind already on the
            // §2.7.7/Q9 parallel-kind track drives the signed/unsigned
            // selection. `EqInt`/`NeqInt` stay kind-agnostic — bitwise
            // equality is identical for `i64` and `u64`.
            GtInt => {
                let (b_bits, b_kind) = self.pop_kinded()?;
                let (a_bits, a_kind) = self.pop_kinded()?;
                let result = if int_cmp_is_unsigned(a_kind, b_kind) {
                    a_bits > b_bits
                } else {
                    (a_bits as i64) > (b_bits as i64)
                };
                self.push_kinded(result as u64, NativeKind::Bool)?;
            }
            LtInt => {
                let (b_bits, b_kind) = self.pop_kinded()?;
                let (a_bits, a_kind) = self.pop_kinded()?;
                let result = if int_cmp_is_unsigned(a_kind, b_kind) {
                    a_bits < b_bits
                } else {
                    (a_bits as i64) < (b_bits as i64)
                };
                self.push_kinded(result as u64, NativeKind::Bool)?;
            }
            GteInt => {
                let (b_bits, b_kind) = self.pop_kinded()?;
                let (a_bits, a_kind) = self.pop_kinded()?;
                let result = if int_cmp_is_unsigned(a_kind, b_kind) {
                    a_bits >= b_bits
                } else {
                    (a_bits as i64) >= (b_bits as i64)
                };
                self.push_kinded(result as u64, NativeKind::Bool)?;
            }
            LteInt => {
                let (b_bits, b_kind) = self.pop_kinded()?;
                let (a_bits, a_kind) = self.pop_kinded()?;
                let result = if int_cmp_is_unsigned(a_kind, b_kind) {
                    a_bits <= b_bits
                } else {
                    (a_bits as i64) <= (b_bits as i64)
                };
                self.push_kinded(result as u64, NativeKind::Bool)?;
            }
            EqInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                self.push_kinded(
                    ((a_bits as i64) == (b_bits as i64)) as u64,
                    NativeKind::Bool,
                )?;
            }
            NeqInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                self.push_kinded(
                    ((a_bits as i64) != (b_bits as i64)) as u64,
                    NativeKind::Bool,
                )?;
            }
            // ===== Number family — kind-aware coercion (Float64 fast, Int promote) =====
            //
            // Wave 6.5: kind-aware comparison of Float64 / Int family
            // operands. The pre-Wave-6 dual-path detector is gone; we now
            // dispatch on the popped kind directly.
            GtNumber => self.cmp_number_kinded(|a, b| a > b)?,
            LtNumber => self.cmp_number_kinded(|a, b| a < b)?,
            GteNumber => self.cmp_number_kinded(|a, b| a >= b)?,
            LteNumber => self.cmp_number_kinded(|a, b| a <= b)?,
            EqNumber => self.cmp_number_kinded(|a, b| a == b)?,
            NeqNumber => self.cmp_number_kinded(|a, b| a != b)?,
            // ===== Decimal family — heap-backed Arc<Decimal> via HeapValue =====
            GtDecimal => self.cmp_decimal_kinded(|a, b| a > b)?,
            LtDecimal => self.cmp_decimal_kinded(|a, b| a < b)?,
            GteDecimal => self.cmp_decimal_kinded(|a, b| a >= b)?,
            LteDecimal => self.cmp_decimal_kinded(|a, b| a <= b)?,
            EqDecimal => self.cmp_decimal_kinded(|a, b| a == b)?,
            // ===== String family — heap-backed Arc<String> via NativeKind::String =====
            GtString => self.cmp_string_kinded(|a, b| a > b)?,
            LtString => self.cmp_string_kinded(|a, b| a < b)?,
            GteString => self.cmp_string_kinded(|a, b| a >= b)?,
            LteString => self.cmp_string_kinded(|a, b| a <= b)?,
            EqString => self.cmp_string_eq_kinded()?,
            EqTypedObject => self.cmp_typed_object_kinded()?,
            // ===== Stage 2.6.5.1: typed absence check (IsNull) =====
            //
            // Wave 6.5: pops one slot, releases its share via
            // `drop_with_kind`, pushes `NativeKind::Bool` indicating
            // whether the value was the null/unit sentinel.
            IsNull => {
                let (bits, kind) = self.pop_kinded()?;
                let is_absent = is_null_kinded(bits, kind);
                drop_with_kind(bits, kind);
                self.push_kinded(is_absent as u64, NativeKind::Bool)?;
            }
            _ => unreachable!(
                "exec_typed_comparison called with non-typed-comparison opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    /// Number-family comparison: pops two operands, coerces each via the
    /// kinded numeric domain (Int family → f64, Float64 → f64), applies
    /// `cmp` and pushes a `NativeKind::Bool` result.
    #[inline(always)]
    fn cmp_number_kinded(&mut self, cmp: impl FnOnce(f64, f64) -> bool) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let af = numeric_as_f64(a_bits, a_kind).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: kind_type_name(a_kind),
        });
        let bf = numeric_as_f64(b_bits, b_kind).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: kind_type_name(b_kind),
        });
        // Release operand shares (Number/Int are inline scalars; drop is a no-op
        // but we keep the call for symmetry/safety in case kind is heap-backed).
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        let result = cmp(af?, bf?);
        self.push_kinded(result as u64, NativeKind::Bool)
    }

    /// Decimal comparison: pops two slots expecting `Ptr(HeapKind::Decimal)`,
    /// dispatches through `as_heap_value()` to read the underlying
    /// `Arc<Decimal>` per ADR-005 §1 single-discriminator, applies the
    /// comparator, releases both shares, pushes `NativeKind::Bool`.
    #[inline(always)]
    fn cmp_decimal_kinded(
        &mut self,
        cmp: impl FnOnce(&rust_decimal::Decimal, &rust_decimal::Decimal) -> bool,
    ) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let result = match (decimal_ref(a_bits, a_kind), decimal_ref(b_bits, b_kind)) {
            (Some(ad), Some(bd)) => cmp(ad, bd),
            _ => false,
        };
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        self.push_kinded(result as u64, NativeKind::Bool)
    }

    /// String ordered comparison: pops two slots expecting `NativeKind::String`,
    /// applies the comparator on the borrowed `&str`, releases shares,
    /// pushes `NativeKind::Bool`.
    #[inline(always)]
    fn cmp_string_kinded(&mut self, cmp: impl FnOnce(&str, &str) -> bool) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let result = cmp(
            str_ref(a_bits, a_kind).unwrap_or(""),
            str_ref(b_bits, b_kind).unwrap_or(""),
        );
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        self.push_kinded(result as u64, NativeKind::Bool)
    }

    /// String equality with mixed Char-vs-String tolerance (string indexing
    /// returns `Char`). Pops two slots, attempts string-string comparison
    /// then falls back to char-char or mixed char/single-char-string.
    #[inline(always)]
    fn cmp_string_eq_kinded(&mut self) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let a_str = str_ref(a_bits, a_kind);
        let b_str = str_ref(b_bits, b_kind);
        let a_char = char_value(a_bits, a_kind);
        let b_char = char_value(b_bits, b_kind);
        let eq = match (a_str, b_str) {
            (Some(asr), Some(bsr)) => asr == bsr,
            (Some(asr), None) => b_char.is_some_and(|c| {
                let mut buf = [0u8; 4];
                asr == c.encode_utf8(&mut buf)
            }),
            (None, Some(bsr)) => a_char.is_some_and(|c| {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf) == bsr
            }),
            (None, None) => match (a_char, b_char) {
                (Some(ac), Some(bc)) => ac == bc,
                _ => false,
            },
        };
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        self.push_kinded(eq as u64, NativeKind::Bool)
    }

    /// Typed object equality for compiler-proven same-schema operands.
    #[inline(always)]
    fn cmp_typed_object_kinded(&mut self) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let result = match (a_kind, b_kind) {
            (NativeKind::Ptr(HeapKind::TypedObject), NativeKind::Ptr(HeapKind::TypedObject)) => {
                typed_object_storage_eq(
                    a_bits as *const TypedObjectStorage,
                    b_bits as *const TypedObjectStorage,
                )
            }
            _ => {
                drop_with_kind(a_bits, a_kind);
                drop_with_kind(b_bits, b_kind);
                return Err(VMError::TypeError {
                    expected: "typed object",
                    got: "non-typed-object operand",
                });
            }
        };
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        self.push_kinded(result as u64, NativeKind::Bool)
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Module-level helpers — read-by-(bits,kind) without consuming the share
// ────────────────────────────────────────────────────────────────────────────

/// Whether a `*Int` ordering comparison (`GtInt`/`LtInt`/`GteInt`/`LteInt`)
/// must use UNSIGNED `u64` semantics rather than signed `i64`.
///
/// R5c-2-β-γ checkpoint (b) u64-carrier. The only integer kind where the
/// signed and unsigned interpretations of the 64-bit slot bits genuinely
/// diverge for ordering is `u64`: a `u64` value above `i64::MAX` has bit 63
/// set, which signed comparison reads as a negative number. `u8`/`u16`/`u32`
/// values are always non-negative in `i64` (they occupy only the low
/// 8/16/32 bits), so signed comparison is already correct for them — and
/// `i8`..`i64` are signed by definition. A comparison is unsigned when
/// EITHER operand is the full-range `u64` carrier; the other operand may be
/// an `Int64`-stamped width-polymorphic literal (a `u64` literal `<=
/// i64::MAX` pushes `Constant::Int` → `NativeKind::Int64`), and an unsigned
/// comparison is still correct for it because such a literal's bits have
/// bit 63 clear.
#[inline]
fn int_cmp_is_unsigned(a_kind: NativeKind, b_kind: NativeKind) -> bool {
    matches!(a_kind, NativeKind::UInt64 | NativeKind::NullableUInt64)
        || matches!(b_kind, NativeKind::UInt64 | NativeKind::NullableUInt64)
}

#[inline]
fn typed_object_storage_eq(a: *const TypedObjectStorage, b: *const TypedObjectStorage) -> bool {
    if a == b {
        return true;
    }
    if a.is_null() || b.is_null() {
        return false;
    }

    // SAFETY: EqTypedObject is emitted only for expressions proven to be typed
    // objects. Slots own live shares until drop_with_kind runs after compare.
    let (a, b) = unsafe { (&*a, &*b) };
    let (a_slots, b_slots) = (a.slots(), b.slots());
    a.schema_id == b.schema_id
        && a.heap_mask == b.heap_mask
        && a.field_kinds.as_ref() == b.field_kinds.as_ref()
        && a_slots.len() == b_slots.len()
        && a.field_kinds.len() == a_slots.len()
        && a_slots
            .iter()
            .zip(b_slots)
            .zip(a.field_kinds.iter())
            .all(|((a, b), kind)| typed_object_field_eq(a.raw(), b.raw(), *kind))
}

#[inline]
fn typed_object_field_eq(a_bits: u64, b_bits: u64, kind: NativeKind) -> bool {
    match kind {
        NativeKind::Null => true,
        NativeKind::String | NativeKind::StringV2 => {
            match (str_ref(a_bits, kind), str_ref(b_bits, kind)) {
                (Some(a), Some(b)) => a == b,
                (None, None) => true,
                _ => false,
            }
        }
        NativeKind::DecimalV2 | NativeKind::Ptr(HeapKind::Decimal) => {
            match (decimal_ref(a_bits, kind), decimal_ref(b_bits, kind)) {
                (Some(a), Some(b)) => a == b,
                (None, None) => true,
                _ => false,
            }
        }
        NativeKind::Ptr(HeapKind::TypedObject) => typed_object_storage_eq(
            a_bits as *const TypedObjectStorage,
            b_bits as *const TypedObjectStorage,
        ),
        NativeKind::Ptr(HeapKind::Char) => a_bits == b_bits,
        NativeKind::Ptr(
            HeapKind::String
            | HeapKind::Closure
            | HeapKind::BigInt
            | HeapKind::DataTable
            | HeapKind::Future
            | HeapKind::TaskGroup
            | HeapKind::TypedArray
            | HeapKind::Temporal
            | HeapKind::TableView
            | HeapKind::Content
            | HeapKind::Instant
            | HeapKind::IoHandle
            | HeapKind::NativeScalar
            | HeapKind::NativeView
            | HeapKind::HashMap
            | HeapKind::FilterExpr
            | HeapKind::Reference
            | HeapKind::SharedCell
            | HeapKind::HashSet
            | HeapKind::Iterator
            | HeapKind::Deque
            | HeapKind::Channel
            | HeapKind::PriorityQueue
            | HeapKind::Range
            | HeapKind::Result
            | HeapKind::Option
            | HeapKind::TraitObject
            | HeapKind::Mutex
            | HeapKind::Atomic
            | HeapKind::Lazy
            | HeapKind::ModuleFn
            | HeapKind::Matrix
            | HeapKind::MatrixSlice,
        ) => a_bits == b_bits,
        NativeKind::Float64
        | NativeKind::NullableFloat64
        | NativeKind::Float32
        | NativeKind::Char
        | NativeKind::Int8
        | NativeKind::NullableInt8
        | NativeKind::UInt8
        | NativeKind::NullableUInt8
        | NativeKind::Int16
        | NativeKind::NullableInt16
        | NativeKind::UInt16
        | NativeKind::NullableUInt16
        | NativeKind::Int32
        | NativeKind::NullableInt32
        | NativeKind::UInt32
        | NativeKind::NullableUInt32
        | NativeKind::Int64
        | NativeKind::NullableInt64
        | NativeKind::UInt64
        | NativeKind::NullableUInt64
        | NativeKind::IntSize
        | NativeKind::NullableIntSize
        | NativeKind::UIntSize
        | NativeKind::NullableUIntSize
        | NativeKind::Bool => a_bits == b_bits,
    }
}

/// Read a `KindedSlot`-style operand as `i128` if it is integer-family
/// (signed/unsigned, any width). Returns `None` for non-integer kinds.
#[inline]
fn numeric_as_i128(bits: u64, kind: NativeKind) -> Option<i128> {
    match kind {
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize => Some((bits as i64) as i128),
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => Some(bits as i128),
        NativeKind::Ptr(HeapKind::BigInt) => {
            let hv = unsafe { &*(bits as *const HeapValue) };
            if let HeapValue::BigInt(arc) = hv {
                Some(**arc as i128)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Read a `KindedSlot`-style operand as `f64` if it is `Float64` or
/// integer-family (with lossless widening).
#[inline]
fn numeric_as_f64(bits: u64, kind: NativeKind) -> Option<f64> {
    match kind {
        NativeKind::Float64 | NativeKind::NullableFloat64 => Some(f64::from_bits(bits)),
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize => Some(bits as i64 as f64),
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => Some(bits as f64),
        _ => None,
    }
}

/// Read a `KindedSlot`-style operand as a borrowed `&Decimal` from either
/// decimal carrier. Mirrors `arithmetic::decimal_ref` — see that function's
/// doc for the two statically-distinct, compile-time-stamped carriers:
///
///   - `NativeKind::Ptr(HeapKind::Decimal)` — `Arc<Decimal>`, `Decimal` at
///     offset 0 (the stack stores the `Arc::into_raw` pointer directly;
///     matches `KindedSlot::from_decimal`). NOT a `*const HeapValue` —
///     slots store `Arc::into_raw(Arc<rust_decimal::Decimal>)`.
///   - `NativeKind::DecimalV2` — `*const DecimalObj`, `Decimal` inline at
///     `DecimalObj::OFFSET_VALUE` (8). Produced by typed-array element reads.
///
/// Recognizing both keeps decimal comparison correct when an operand comes
/// from a typed-array element. Recognizes the proven carrier by its stamped
/// kind; does NOT reinterpret scalar bits as a pointer.
#[inline]
fn decimal_ref<'a>(bits: u64, kind: NativeKind) -> Option<&'a rust_decimal::Decimal> {
    if bits == 0 {
        return None;
    }
    match kind {
        NativeKind::Ptr(HeapKind::Decimal) => {
            let ptr = bits as *const rust_decimal::Decimal;
            Some(unsafe { &*ptr })
        }
        NativeKind::DecimalV2 => {
            let value_ptr = (bits as *const u8)
                .wrapping_add(shape_value::v2::decimal_obj::DecimalObj::OFFSET_VALUE)
                as *const rust_decimal::Decimal;
            Some(unsafe { &*value_ptr })
        }
        _ => None,
    }
}

/// Borrowed-decimal helper consumed by `nb_compare_numeric_kinded`.
#[inline]
fn numeric_as_decimal_ref<'a>(bits: u64, kind: NativeKind) -> Option<&'a rust_decimal::Decimal> {
    decimal_ref(bits, kind)
}

/// Read a `KindedSlot`-style operand as a borrowed `&str`.
///
/// Accepts BOTH string carriers — `NativeKind::String` (Phase-2c
/// `Arc<String>` carrier; ADR-005 §2 String exception) AND
/// `NativeKind::StringV2` (Wave 2 Agent B v2-raw `*const StringObj` carrier
/// per ADR-006 §2.7.5 amendment) — equating them at the comparison-shell
/// boundary. WS-8 (2026-05-22): the post-fix `cmp_string_eq_kinded` then
/// compares the resulting `&str` slices for both directions, so
/// `let xs = ["a", "b"]; xs.includes("a")` (where the array-iterated element
/// arrives as `StringV2` and the literal `"a"` arrives as `String`) returns
/// the correct `true`. The slot owns the per-carrier share; the borrow is
/// valid for the lifetime of the slot.
#[inline]
fn str_ref<'a>(bits: u64, kind: NativeKind) -> Option<&'a str> {
    if bits == 0 {
        return None;
    }
    match kind {
        NativeKind::String => {
            let ptr = bits as *const String;
            Some(unsafe { (*ptr).as_str() })
        }
        NativeKind::StringV2 => {
            let ptr = bits as usize as *const shape_value::v2::string_obj::StringObj;
            Some(unsafe { shape_value::v2::string_obj::StringObj::as_str(ptr) })
        }
        _ => None,
    }
}

/// Read a `KindedSlot`-style operand as a `char` if the kind is
/// `Ptr(HeapKind::Char)` (Char is an inline-codepoint payload tagged
/// through HeapKind for dispatch uniformity).
#[inline]
fn char_value(bits: u64, kind: NativeKind) -> Option<char> {
    if !matches!(kind, NativeKind::Ptr(HeapKind::Char)) {
        return None;
    }
    char::from_u32(bits as u32)
}

/// Exhaustive HeapKind sink for pointer-null checks.
#[inline]
fn heap_ptr_is_null(bits: u64, heap_kind: HeapKind) -> bool {
    match heap_kind {
        HeapKind::String
        | HeapKind::TypedObject
        | HeapKind::Closure
        | HeapKind::Decimal
        | HeapKind::BigInt
        | HeapKind::DataTable
        | HeapKind::Future
        | HeapKind::TaskGroup
        | HeapKind::TypedArray
        | HeapKind::Temporal
        | HeapKind::TableView
        | HeapKind::Content
        | HeapKind::Instant
        | HeapKind::IoHandle
        | HeapKind::NativeScalar
        | HeapKind::NativeView
        | HeapKind::Char
        | HeapKind::HashMap
        | HeapKind::FilterExpr
        | HeapKind::Reference
        | HeapKind::SharedCell
        | HeapKind::HashSet
        | HeapKind::Iterator
        | HeapKind::Deque
        | HeapKind::Channel
        | HeapKind::PriorityQueue
        | HeapKind::Range
        | HeapKind::Result
        | HeapKind::Option
        | HeapKind::TraitObject
        | HeapKind::Mutex
        | HeapKind::Atomic
        | HeapKind::Lazy
        | HeapKind::ModuleFn
        | HeapKind::Matrix
        | HeapKind::MatrixSlice => bits == 0,
    }
}

/// Test whether a `(bits, kind)` pair encodes the null/unit sentinel.
///
/// R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 + §2.7.5 + §2.7.7/Q9,
/// 2026-05-19): pre-disposition `(0u64, NativeKind::Bool)` was the
/// canonical null sentinel; SURFACE-G6-BOOL-NULL surfaced this colliding
/// with legitimate `false` bool values (both encoded as bits=0). The
/// `bool`-default parameter fill-in path emitted `LoadLocal + IsNull +
/// JumpIfFalse + StoreLocal(default)` and mis-detected `check(false)`
/// as "caller omitted; fill default", causing VM-only divergence vs the
/// JIT path. Post-disposition: `NativeKind::Bool` slots NEVER carry the
/// null sentinel — kind IS the discriminator per §2.7.7/Q9. Null is
/// pushed with `NativeKind::Null` discriminator at `PushNull` +
/// `Constant::Null` + `Constant::Unit` producer sites.
#[inline]
fn is_null_kinded(bits: u64, kind: NativeKind) -> bool {
    match kind {
        // R5b-2 disposition: Null IS the absence-of-value discriminator;
        // kind alone is decisive, bits unused.
        NativeKind::Null => true,
        // R5b-2 disposition: Bool slots carry only `{0, 1}` bit
        // patterns for real bool values — `false` is NOT null.
        NativeKind::Bool => false,
        NativeKind::String => bits == 0,
        NativeKind::Ptr(heap_kind) => heap_ptr_is_null(bits, heap_kind),
        NativeKind::NullableFloat64 => f64::from_bits(bits).is_nan(),
        NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => bits == 0,
        // Non-nullable scalar kinds are never null.
        _ => false,
    }
}

/// `&'static str` description of a `NativeKind` for `VMError::TypeError`.
#[inline]
fn kind_type_name(kind: NativeKind) -> &'static str {
    match kind {
        // R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 + §2.7.7/Q9,
        // 2026-05-19): canonical absence-of-value discriminator.
        NativeKind::Null => "null",
        NativeKind::Bool => "bool",
        NativeKind::Float64 | NativeKind::NullableFloat64 => "number",
        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
        // ADR-006 §2.7.5 amendment adds F32 + Char as scalar variants.
        NativeKind::Float32 => "f32",
        NativeKind::Char => "char",
        // Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-additions
        // (2026-05-14): same surface as Arc-wrapped siblings.
        NativeKind::StringV2 => "string",
        NativeKind::DecimalV2 => "decimal",
        NativeKind::Int8 | NativeKind::NullableInt8 => "i8",
        NativeKind::Int16 | NativeKind::NullableInt16 => "i16",
        NativeKind::Int32 | NativeKind::NullableInt32 => "i32",
        NativeKind::Int64 | NativeKind::NullableInt64 => "int",
        NativeKind::IntSize | NativeKind::NullableIntSize => "isize",
        NativeKind::UInt8 | NativeKind::NullableUInt8 => "u8",
        NativeKind::UInt16 | NativeKind::NullableUInt16 => "u16",
        NativeKind::UInt32 | NativeKind::NullableUInt32 => "u32",
        NativeKind::UInt64 | NativeKind::NullableUInt64 => "u64",
        NativeKind::UIntSize | NativeKind::NullableUIntSize => "usize",
        NativeKind::String => "string",
        NativeKind::Ptr(HeapKind::String) => "string",
        NativeKind::Ptr(HeapKind::TypedArray) => "array",
        NativeKind::Ptr(HeapKind::TypedObject) => "object",
        NativeKind::Ptr(HeapKind::HashMap) => "map",
        NativeKind::Ptr(HeapKind::Decimal) => "decimal",
        NativeKind::Ptr(HeapKind::BigInt) => "bigint",
        NativeKind::Ptr(HeapKind::DataTable) => "table",
        NativeKind::Ptr(HeapKind::IoHandle) => "io_handle",
        NativeKind::Ptr(HeapKind::NativeView) => "native_view",
        NativeKind::Ptr(HeapKind::Content) => "content",
        NativeKind::Ptr(HeapKind::Instant) => "instant",
        NativeKind::Ptr(HeapKind::Temporal) => "temporal",
        NativeKind::Ptr(HeapKind::TableView) => "table_view",
        NativeKind::Ptr(HeapKind::TaskGroup) => "task_group",
        NativeKind::Ptr(HeapKind::Char) => "char",
        NativeKind::Ptr(HeapKind::Closure) => "closure",
        NativeKind::Ptr(HeapKind::Future) => "future",
        NativeKind::Ptr(HeapKind::NativeScalar) => "native_scalar",
        // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / Q8 amendment): the
        // FilterExpr discriminator labels query-DSL Arc<FilterNode>
        // payloads emitted by `executor/logical/mod.rs`.
        NativeKind::Ptr(HeapKind::FilterExpr) => "filter_expr",
        // ADR-006 §2.7.13 / Q14 (Wave 8 W8-T26): Reference carriers
        // (`Arc<RefTarget>`) emitted by the `MakeRef` family.
        NativeKind::Ptr(HeapKind::Reference) => "ref",
        // Wave 8 W8-T25 (ADR-006 §2.7.12 / Q13 amendment, 2026-05-10):
        // `Arc<SharedCell>` cell-pointer slots emitted by
        // `op_alloc_shared_local` / `op_alloc_shared_module_binding`.
        NativeKind::Ptr(HeapKind::SharedCell) => "shared_cell",
        // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16, 2026-05-10).
        NativeKind::Ptr(HeapKind::HashSet) => "set",
        // W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10):
        // `Arc<IteratorState>` lazy-iterator carriers emitted by the
        // iterator-method PHF.
        NativeKind::Ptr(HeapKind::Iterator) => "iterator",
        // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20, 2026-05-10):
        // `Arc<DequeData>` double-ended-queue carriers emitted by
        // the Deque ctor + DEQUE_METHODS PHF.
        NativeKind::Ptr(HeapKind::Deque) => "deque",
        // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21, 2026-05-10):
        // `Arc<ChannelData>` MPSC channel carriers emitted by `Channel()`
        // ctor + the CHANNEL_METHODS PHF.
        NativeKind::Ptr(HeapKind::Channel) => "channel",
        // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19,
        // 2026-05-10): `Arc<PriorityQueueData>` min-heap carriers
        // emitted by the `PriorityQueueCtor` ctor.
        NativeKind::Ptr(HeapKind::PriorityQueue) => "priority_queue",
        // W15-range (ADR-006 §2.7.23 / Q24, 2026-05-10):
        // `Arc<RangeData>` range-value carriers emitted by `MakeRange`.
        NativeKind::Ptr(HeapKind::Range) => "range",
        // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18, 2026-05-10).
        NativeKind::Ptr(HeapKind::Result) => "result",
        NativeKind::Ptr(HeapKind::Option) => "option",
        // W17-concurrency (ADR-006 §2.7.25, 2026-05-11):
        // `Arc<MutexData>` / `Arc<AtomicData>` / `Arc<LazyData>`
        // concurrency-primitive carriers emitted by the Mutex/Atomic/
        // Lazy ctors + MUTEX_METHODS / ATOMIC_METHODS / LAZY_METHODS
        // PHFs.
        NativeKind::Ptr(HeapKind::Mutex) => "mutex",
        NativeKind::Ptr(HeapKind::Atomic) => "atomic",
        NativeKind::Ptr(HeapKind::Lazy) => "lazy",
        // W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C, 2026-05-11):
        // `Arc<TraitObjectStorage>` carrier for `dyn Trait`. Compared
        // values surface the carrier's display name; user-level
        // equality goes through trait-method dispatch (Eq trait).
        NativeKind::Ptr(HeapKind::TraitObject) => "trait_object",
        // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12):
        // ModuleFn references — inline-scalar module-fn-id label.
        NativeKind::Ptr(HeapKind::ModuleFn) => "module_fn",
        // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13).
        NativeKind::Ptr(HeapKind::Matrix) => "matrix",
        NativeKind::Ptr(HeapKind::MatrixSlice) => "matrix_slice",
    }
}

// Re-export the kinded compare for callers that previously used
// `nb_compare_numeric` on a pair of `&ValueWord`s. New name-shape uses
// `(bits, kind)` pairs to match the post-§2.7.7 ABI.
//
// (Kept unused as a stable internal symbol for downstream wave migrations
// that need cross-numeric ordering at the body site.)
#[allow(dead_code)]
fn _expose(a_bits: u64, a_kind: NativeKind, b_bits: u64, b_kind: NativeKind) -> Option<Ordering> {
    VirtualMachine::nb_compare_numeric_kinded(a_bits, a_kind, b_bits, b_kind)
}

// Allow the Wave-6 import-pruning to skip warnings on unused-yet-stable
// re-exports (Arc / ValueSlot may be referenced by future test modules).
#[allow(unused_imports)]
use Arc as _Arc;
#[allow(unused_imports)]
use ValueSlot as _ValueSlot;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Instruction;
    use crate::executor::{VMConfig, VirtualMachine};

    fn make_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    fn run_typed_cmp(vm: &mut VirtualMachine, opcode: OpCode) -> bool {
        let instr = Instruction {
            opcode,
            operand: None,
        };
        vm.exec_typed_comparison(&instr).unwrap();
        // Wave 6.5: comparison handlers push `NativeKind::Bool` — read via
        // pop_kinded.
        let (bits, kind) = vm.pop_kinded().unwrap();
        assert_eq!(kind, NativeKind::Bool, "comparison must produce Bool kind");
        bits != 0
    }

    // ----- Int comparison -----

    #[test]
    fn typed_int_eq() {
        let mut vm = make_vm();
        vm.push_kinded(42u64, NativeKind::Int64).unwrap();
        vm.push_kinded(42u64, NativeKind::Int64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqInt));
    }

    #[test]
    fn typed_int_neq() {
        let mut vm = make_vm();
        vm.push_kinded(1u64, NativeKind::Int64).unwrap();
        vm.push_kinded(2u64, NativeKind::Int64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::NeqInt));
    }

    #[test]
    fn typed_int_lt() {
        let mut vm = make_vm();
        vm.push_kinded((-5i64) as u64, NativeKind::Int64).unwrap();
        vm.push_kinded(3u64, NativeKind::Int64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::LtInt));
    }

    #[test]
    fn typed_int_gt() {
        let mut vm = make_vm();
        vm.push_kinded(7u64, NativeKind::Int64).unwrap();
        vm.push_kinded(3u64, NativeKind::Int64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::GtInt));
    }

    #[test]
    fn typed_int_gte_lte_boundary_equal() {
        let mut vm = make_vm();
        vm.push_kinded(10u64, NativeKind::Int64).unwrap();
        vm.push_kinded(10u64, NativeKind::Int64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::GteInt));
        let mut vm = make_vm();
        vm.push_kinded(10u64, NativeKind::Int64).unwrap();
        vm.push_kinded(10u64, NativeKind::Int64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::LteInt));
    }

    // ----- Number comparison -----

    #[test]
    fn typed_number_eq() {
        let mut vm = make_vm();
        vm.push_kinded(1.5f64.to_bits(), NativeKind::Float64)
            .unwrap();
        vm.push_kinded(1.5f64.to_bits(), NativeKind::Float64)
            .unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqNumber));
    }

    #[test]
    fn typed_number_lt() {
        let mut vm = make_vm();
        vm.push_kinded((-1.0f64).to_bits(), NativeKind::Float64)
            .unwrap();
        vm.push_kinded(0.5f64.to_bits(), NativeKind::Float64)
            .unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::LtNumber));
    }

    #[test]
    fn typed_number_gt() {
        let mut vm = make_vm();
        vm.push_kinded(3.14f64.to_bits(), NativeKind::Float64)
            .unwrap();
        vm.push_kinded(2.71f64.to_bits(), NativeKind::Float64)
            .unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::GtNumber));
    }

    // ----- NaN semantics -----

    #[test]
    fn typed_number_eq_nan_is_false() {
        let mut vm = make_vm();
        vm.push_kinded(f64::NAN.to_bits(), NativeKind::Float64)
            .unwrap();
        vm.push_kinded(f64::NAN.to_bits(), NativeKind::Float64)
            .unwrap();
        assert!(!run_typed_cmp(&mut vm, OpCode::EqNumber));
    }

    #[test]
    fn typed_number_neq_nan_is_true() {
        let mut vm = make_vm();
        vm.push_kinded(f64::NAN.to_bits(), NativeKind::Float64)
            .unwrap();
        vm.push_kinded(f64::NAN.to_bits(), NativeKind::Float64)
            .unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::NeqNumber));
    }

    #[test]
    fn typed_number_lt_nan_is_false() {
        let mut vm = make_vm();
        vm.push_kinded(1.0f64.to_bits(), NativeKind::Float64)
            .unwrap();
        vm.push_kinded(f64::NAN.to_bits(), NativeKind::Float64)
            .unwrap();
        assert!(!run_typed_cmp(&mut vm, OpCode::LtNumber));
    }

    #[test]
    fn typed_number_gt_nan_is_false() {
        let mut vm = make_vm();
        vm.push_kinded(1.0f64.to_bits(), NativeKind::Float64)
            .unwrap();
        vm.push_kinded(f64::NAN.to_bits(), NativeKind::Float64)
            .unwrap();
        assert!(!run_typed_cmp(&mut vm, OpCode::GtNumber));
    }

    #[test]
    fn typed_number_eq_treats_neg_zero_as_zero() {
        let mut vm = make_vm();
        vm.push_kinded((-0.0f64).to_bits(), NativeKind::Float64)
            .unwrap();
        vm.push_kinded((0.0f64).to_bits(), NativeKind::Float64)
            .unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqNumber));
    }

    // ----- IsNull -----

    fn run_is_null(vm: &mut VirtualMachine) -> bool {
        let instr = Instruction {
            opcode: OpCode::IsNull,
            operand: None,
        };
        vm.exec_typed_comparison(&instr).unwrap();
        let (bits, kind) = vm.pop_kinded().unwrap();
        assert_eq!(kind, NativeKind::Bool);
        bits != 0
    }

    /// R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 + §2.7.5 +
    /// §2.7.7/Q9, 2026-05-19): post-disposition `NativeKind::Null` is
    /// the canonical absence-of-value discriminator; kind alone is
    /// decisive, bits unused.
    #[test]
    fn is_null_on_null_kind_returns_true() {
        let mut vm = make_vm();
        vm.push_kinded(0u64, NativeKind::Null).unwrap();
        assert!(run_is_null(&mut vm));
    }

    #[test]
    fn is_null_on_int_returns_false() {
        let mut vm = make_vm();
        vm.push_kinded(42u64, NativeKind::Int64).unwrap();
        assert!(!run_is_null(&mut vm));
    }

    #[test]
    fn is_null_on_zero_int_returns_false() {
        // int 0 is NOT null — kind discriminates the int-zero literal
        // from the null sentinel (post-R5b-2 the discriminator is
        // `NativeKind::Null`, not `(NativeKind::Bool, bits=0)`).
        let mut vm = make_vm();
        vm.push_kinded(0u64, NativeKind::Int64).unwrap();
        assert!(!run_is_null(&mut vm));
    }

    /// R5b-2-bool-null-sentinel-cluster regression pin (ADR-006 §2.7 +
    /// §2.7.5 + §2.7.7/Q9, 2026-05-19): post-disposition Bool slots
    /// carry only legitimate `{0, 1}` bool bit patterns — `false` is
    /// NOT null. SURFACE-G6-BOOL-NULL pin: pre-disposition
    /// `is_null_kinded(0, NativeKind::Bool) == true` caused
    /// `check(false)` for `fn check(val: bool = true)` to mis-detect
    /// false as null and fill the default value (returning `true`
    /// instead of `false`).
    #[test]
    fn is_null_on_false_bool_returns_false_post_r5b2() {
        let mut vm = make_vm();
        vm.push_kinded(0u64, NativeKind::Bool).unwrap();
        assert!(!run_is_null(&mut vm));
    }

    #[test]
    fn is_null_on_true_bool_returns_false() {
        let mut vm = make_vm();
        vm.push_kinded(1u64, NativeKind::Bool).unwrap();
        assert!(!run_is_null(&mut vm));
    }

    // ----- nb_compare_numeric_kinded direct-API tests -----

    #[test]
    fn compare_numeric_kinded_handles_int_int() {
        assert_eq!(
            VirtualMachine::nb_compare_numeric_kinded(
                7u64,
                NativeKind::Int64,
                3u64,
                NativeKind::Int64
            ),
            Some(Ordering::Greater),
        );
    }

    #[test]
    fn compare_numeric_kinded_handles_float_float() {
        assert_eq!(
            VirtualMachine::nb_compare_numeric_kinded(
                1.0f64.to_bits(),
                NativeKind::Float64,
                2.0f64.to_bits(),
                NativeKind::Float64
            ),
            Some(Ordering::Less),
        );
    }

    #[test]
    fn compare_numeric_kinded_int_vs_float_lossless() {
        assert_eq!(
            VirtualMachine::nb_compare_numeric_kinded(
                5u64,
                NativeKind::Int64,
                5.0f64.to_bits(),
                NativeKind::Float64
            ),
            Some(Ordering::Equal),
        );
    }

    // ── u64 ordering comparisons — R5c-2-β-γ checkpoint (b) ───────────────
    //
    // `*Int` ordering opcodes (`GtInt`/`LtInt`/`GteInt`/`LteInt`) are
    // signedness-DEPENDENT. The pre-checkpoint-(b) handlers reinterpreted
    // the 64-bit slot bits as signed `i64` unconditionally, so a
    // `NativeKind::UInt64` operand above `i64::MAX` (bit 63 set) compared
    // as a negative number — `u64::MAX > 2` evaluated `false`. The handler
    // now consults the producer-stamped operand kind (`int_cmp_is_unsigned`)
    // and uses an unsigned comparison when either operand is `UInt64`.

    /// Push two `u64` operands with the `UInt64` carrier kind, run the
    /// opcode, return the bool result.
    fn run_u64_cmp(a: u64, b: u64, opcode: OpCode) -> bool {
        let mut vm = make_vm();
        vm.push_kinded(a, NativeKind::UInt64).unwrap();
        vm.push_kinded(b, NativeKind::UInt64).unwrap();
        run_typed_cmp(&mut vm, opcode)
    }

    #[test]
    fn u64_gt_above_i64_max_is_greater() {
        // u64::MAX > 2 — true (unsigned). Signed would give false.
        assert!(run_u64_cmp(u64::MAX, 2, OpCode::GtInt));
    }

    #[test]
    fn u64_lt_above_i64_max_is_not_less() {
        // u64::MAX < 2 — false (unsigned). Signed would give true.
        assert!(!run_u64_cmp(u64::MAX, 2, OpCode::LtInt));
    }

    #[test]
    fn u64_gte_equal_full_range() {
        assert!(run_u64_cmp(u64::MAX, u64::MAX, OpCode::GteInt));
    }

    #[test]
    fn u64_lte_full_range() {
        // (2^63) <= u64::MAX — true; both above i64::MAX.
        assert!(run_u64_cmp(1u64 << 63, u64::MAX, OpCode::LteInt));
    }

    #[test]
    fn u64_eq_full_range_is_bit_exact() {
        // Equality is signedness-agnostic — bit-exact.
        assert!(run_u64_cmp(u64::MAX, u64::MAX, OpCode::EqInt));
        assert!(run_u64_cmp(u64::MAX, u64::MAX - 1, OpCode::NeqInt));
    }

    #[test]
    fn u64_mixed_with_int64_literal_operand_uses_unsigned() {
        // `a` is UInt64-kinded, `b` is an Int64-stamped width-polymorphic
        // literal (a `u64` literal <= i64::MAX pushes Constant::Int →
        // NativeKind::Int64). The comparison must still be unsigned.
        let mut vm = make_vm();
        vm.push_kinded(u64::MAX, NativeKind::UInt64).unwrap();
        vm.push_kinded(2u64, NativeKind::Int64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::GtInt));
    }

    #[test]
    fn signed_int_comparison_unaffected() {
        // Plain `int` (Int64) comparisons keep signed semantics — a
        // negative i64 is still less than a positive one.
        let mut vm = make_vm();
        vm.push_kinded((-1i64) as u64, NativeKind::Int64).unwrap();
        vm.push_kinded(2u64, NativeKind::Int64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::LtInt));
    }
}
