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

use crate::{
    bytecode::{Instruction, OpCode},
    executor::vm_impl::stack::drop_with_kind,
    executor::VirtualMachine,
};
use shape_value::{NativeKind, VMError, heap_value::{HeapKind, HeapValue}, ValueSlot};
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
            // dual-path tag probe is gone (deleted in substep-1; would
            // always have returned false on native bits).
            GtInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                self.push_kinded(((a_bits as i64) > (b_bits as i64)) as u64, NativeKind::Bool)?;
            }
            LtInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                self.push_kinded(((a_bits as i64) < (b_bits as i64)) as u64, NativeKind::Bool)?;
            }
            GteInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                self.push_kinded(((a_bits as i64) >= (b_bits as i64)) as u64, NativeKind::Bool)?;
            }
            LteInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                self.push_kinded(((a_bits as i64) <= (b_bits as i64)) as u64, NativeKind::Bool)?;
            }
            EqInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                self.push_kinded(((a_bits as i64) == (b_bits as i64)) as u64, NativeKind::Bool)?;
            }
            NeqInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                self.push_kinded(((a_bits as i64) != (b_bits as i64)) as u64, NativeKind::Bool)?;
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
}

// ────────────────────────────────────────────────────────────────────────────
// Module-level helpers — read-by-(bits,kind) without consuming the share
// ────────────────────────────────────────────────────────────────────────────

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

/// Read a `KindedSlot`-style operand as a borrowed `&Decimal` if the kind
/// is `Ptr(HeapKind::Decimal)`. Dispatches via `HeapValue` per ADR-005 §1
/// (no per-heap-variant accessor on the carrier).
#[inline]
fn decimal_ref<'a>(bits: u64, kind: NativeKind) -> Option<&'a rust_decimal::Decimal> {
    if !matches!(kind, NativeKind::Ptr(HeapKind::Decimal)) || bits == 0 {
        return None;
    }
    // The Wave-6 stack stores the `Arc::into_raw` pointer for `Decimal`
    // directly (matching `KindedSlot::from_decimal`'s `Arc::into_raw`
    // bits). This is NOT a `*const HeapValue` on the Decimal arm —
    // `Decimal` slots store `Arc::into_raw(Arc<rust_decimal::Decimal>)`,
    // not a `Box<HeapValue>` carrier.
    let ptr = bits as *const rust_decimal::Decimal;
    Some(unsafe { &*ptr })
}

/// Borrowed-decimal helper consumed by `nb_compare_numeric_kinded`.
#[inline]
fn numeric_as_decimal_ref<'a>(bits: u64, kind: NativeKind) -> Option<&'a rust_decimal::Decimal> {
    decimal_ref(bits, kind)
}

/// Read a `KindedSlot`-style operand as a borrowed `&str` if the kind is
/// `NativeKind::String`. The slot owns one `Arc<String>` strong-count
/// share, so the borrow is valid for the lifetime of the slot.
#[inline]
fn str_ref<'a>(bits: u64, kind: NativeKind) -> Option<&'a str> {
    if !matches!(kind, NativeKind::String) || bits == 0 {
        return None;
    }
    let ptr = bits as *const String;
    Some(unsafe { (*ptr).as_str() })
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

/// Test whether a `(bits, kind)` pair encodes the null/unit sentinel.
/// `(0u64, NativeKind::Bool)` is the §2.7 sentinel; nullable scalar arms
/// encode null as zero bits; heap arms have null as a zero pointer.
#[inline]
fn is_null_kinded(bits: u64, kind: NativeKind) -> bool {
    match kind {
        NativeKind::Bool => bits == 0, // unit / null sentinel
        NativeKind::String | NativeKind::Ptr(_) => bits == 0,
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
        NativeKind::Bool => "bool",
        NativeKind::Float64 | NativeKind::NullableFloat64 => "number",
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
    }
}

// Re-export the kinded compare for callers that previously used
// `nb_compare_numeric` on a pair of `&ValueWord`s. New name-shape uses
// `(bits, kind)` pairs to match the post-§2.7.7 ABI.
//
// (Kept unused as a stable internal symbol for downstream wave migrations
// that need cross-numeric ordering at the body site.)
#[allow(dead_code)]
fn _expose(
    a_bits: u64,
    a_kind: NativeKind,
    b_bits: u64,
    b_kind: NativeKind,
) -> Option<Ordering> {
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
        let instr = Instruction { opcode, operand: None };
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
        vm.push_kinded(1.5f64.to_bits(), NativeKind::Float64).unwrap();
        vm.push_kinded(1.5f64.to_bits(), NativeKind::Float64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqNumber));
    }

    #[test]
    fn typed_number_lt() {
        let mut vm = make_vm();
        vm.push_kinded((-1.0f64).to_bits(), NativeKind::Float64).unwrap();
        vm.push_kinded(0.5f64.to_bits(), NativeKind::Float64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::LtNumber));
    }

    #[test]
    fn typed_number_gt() {
        let mut vm = make_vm();
        vm.push_kinded(3.14f64.to_bits(), NativeKind::Float64).unwrap();
        vm.push_kinded(2.71f64.to_bits(), NativeKind::Float64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::GtNumber));
    }

    // ----- NaN semantics -----

    #[test]
    fn typed_number_eq_nan_is_false() {
        let mut vm = make_vm();
        vm.push_kinded(f64::NAN.to_bits(), NativeKind::Float64).unwrap();
        vm.push_kinded(f64::NAN.to_bits(), NativeKind::Float64).unwrap();
        assert!(!run_typed_cmp(&mut vm, OpCode::EqNumber));
    }

    #[test]
    fn typed_number_neq_nan_is_true() {
        let mut vm = make_vm();
        vm.push_kinded(f64::NAN.to_bits(), NativeKind::Float64).unwrap();
        vm.push_kinded(f64::NAN.to_bits(), NativeKind::Float64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::NeqNumber));
    }

    #[test]
    fn typed_number_lt_nan_is_false() {
        let mut vm = make_vm();
        vm.push_kinded(1.0f64.to_bits(), NativeKind::Float64).unwrap();
        vm.push_kinded(f64::NAN.to_bits(), NativeKind::Float64).unwrap();
        assert!(!run_typed_cmp(&mut vm, OpCode::LtNumber));
    }

    #[test]
    fn typed_number_gt_nan_is_false() {
        let mut vm = make_vm();
        vm.push_kinded(1.0f64.to_bits(), NativeKind::Float64).unwrap();
        vm.push_kinded(f64::NAN.to_bits(), NativeKind::Float64).unwrap();
        assert!(!run_typed_cmp(&mut vm, OpCode::GtNumber));
    }

    #[test]
    fn typed_number_eq_treats_neg_zero_as_zero() {
        let mut vm = make_vm();
        vm.push_kinded((-0.0f64).to_bits(), NativeKind::Float64).unwrap();
        vm.push_kinded((0.0f64).to_bits(), NativeKind::Float64).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqNumber));
    }

    // ----- IsNull -----

    fn run_is_null(vm: &mut VirtualMachine) -> bool {
        let instr = Instruction { opcode: OpCode::IsNull, operand: None };
        vm.exec_typed_comparison(&instr).unwrap();
        let (bits, kind) = vm.pop_kinded().unwrap();
        assert_eq!(kind, NativeKind::Bool);
        bits != 0
    }

    #[test]
    fn is_null_on_null_sentinel_returns_true() {
        let mut vm = make_vm();
        vm.push_kinded(0u64, NativeKind::Bool).unwrap();
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
        // int 0 is NOT null — kind discriminates from the (Bool, 0) sentinel.
        let mut vm = make_vm();
        vm.push_kinded(0u64, NativeKind::Int64).unwrap();
        assert!(!run_is_null(&mut vm));
    }

    #[test]
    fn is_null_on_false_bool_returns_true_via_zero_bits() {
        // Bool with zero bits is, by §2.7 sentinel convention, the null/unit
        // marker. (Distinct from `Bool` with 1u64 bits = true.)
        let mut vm = make_vm();
        vm.push_kinded(0u64, NativeKind::Bool).unwrap();
        assert!(run_is_null(&mut vm));
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
}
