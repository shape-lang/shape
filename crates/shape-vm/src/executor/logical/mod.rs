//! Logical operations for the VM executor (ADR-006 §2.7.7 / Q9 — kinded stack).
//!
//! Handles: And, Or, Not, NullCoalesce
//!
//! Wave 6: kinds for both operands are now read from the parallel stack
//! kind track via `pop_kinded()`. Filter-expression dispatch (heap path
//! used by query DSL) discriminates on `NativeKind::Ptr(HeapKind::*)`
//! rather than the deleted `is_heap()` probe.
//!
//! Wave-γ G-heap-filter-expr (ADR-006 §2.3 / §2.7.6 / Q8 amendment,
//! 2026-05-09): the filter-expression branch now labels its
//! `Arc::into_raw(Arc<FilterNode>) as u64` payloads as
//! `NativeKind::Ptr(HeapKind::FilterExpr)`. The earlier label
//! `HeapKind::NativeView` collided with `Arc<NativeViewData>` payloads at
//! the `clone_with_kind` / `drop_with_kind` dispatch tables, causing
//! wrong-type retain/release on every And/Or/Not result. Wave-α
//! D-raw-helpers (commit `a27c0e4`) surfaced the gap; Wave-γ
//! G-heap-filter-expr fixes it by adding `HeapKind::FilterExpr` and
//! mirroring the dispatch arm in every `Q8`/Q10 cell-storage and
//! stack-track table.

use crate::executor::objects::raw_helpers;
use crate::{
    bytecode::{Instruction, OpCode},
    executor::vm_impl::stack::drop_with_kind,
    executor::VirtualMachine,
};
use shape_value::{FilterNode, NativeKind, VMError, heap_value::HeapKind};
use std::sync::Arc;

/// Wave 6: heuristic helper. A pushed slot is a runtime-heap-bearing
/// FilterExpr candidate if its `NativeKind` is one of the heap-arms.
/// Inline scalars (Bool, Int, Float, etc.) trivially are not.
#[inline]
fn kind_is_heap(k: NativeKind) -> bool {
    matches!(k, NativeKind::String | NativeKind::Ptr(_))
}

/// Wave 6: bool truthiness from raw bits + kind. Inline-scalar arms read
/// the bits directly; heap arms are non-null → truthy. The deleted
/// `is_truthy()` probe walked tag bits — Wave 6 dispatches on `kind`.
#[inline]
fn kinded_truthy(bits: u64, kind: NativeKind) -> bool {
    match kind {
        NativeKind::Bool => bits != 0,
        NativeKind::Float64 => f64::from_bits(bits) != 0.0,
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => bits != 0,
        // Nullable arms: zero-bits convention = null = falsy. Otherwise
        // delegate to the underlying bits being non-zero.
        NativeKind::NullableFloat64
        | NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => bits != 0,
        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
        // F32 truthy iff `!= 0.0`; Char truthy iff codepoint bits non-zero.
        NativeKind::Float32 => f32::from_bits(bits as u32) != 0.0,
        NativeKind::Char => bits != 0,
        // Heap-bearing kinds: non-null pointer → truthy.
        NativeKind::String | NativeKind::Ptr(_) => bits != 0,
    }
}

impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_logical(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            And => {
                let (b_bits, b_kind) = self.pop_kinded()?;
                let (a_bits, a_kind) = self.pop_kinded()?;
                if kind_is_heap(a_kind) || kind_is_heap(b_kind) {
                    if let (Some(left), Some(right)) = (
                        raw_helpers::extract_filter_expr(a_bits, a_kind),
                        raw_helpers::extract_filter_expr(b_bits, b_kind),
                    ) {
                        let combined = Arc::new(FilterNode::And(
                            Box::new(left.clone()),
                            Box::new(right.clone()),
                        ));
                        // Filter-expr operands consumed; release shares.
                        drop_with_kind(a_bits, a_kind);
                        drop_with_kind(b_bits, b_kind);
                        let raw = Arc::into_raw(combined) as u64;
                        self.push_kinded(raw, NativeKind::Ptr(HeapKind::FilterExpr))?;
                    } else {
                        let r = kinded_truthy(a_bits, a_kind) && kinded_truthy(b_bits, b_kind);
                        drop_with_kind(a_bits, a_kind);
                        drop_with_kind(b_bits, b_kind);
                        self.push_kinded(r as u64, NativeKind::Bool)?;
                    }
                } else {
                    let r = kinded_truthy(a_bits, a_kind) && kinded_truthy(b_bits, b_kind);
                    self.push_kinded(r as u64, NativeKind::Bool)?;
                }
            }
            Or => {
                let (b_bits, b_kind) = self.pop_kinded()?;
                let (a_bits, a_kind) = self.pop_kinded()?;
                if kind_is_heap(a_kind) || kind_is_heap(b_kind) {
                    if let (Some(left), Some(right)) = (
                        raw_helpers::extract_filter_expr(a_bits, a_kind),
                        raw_helpers::extract_filter_expr(b_bits, b_kind),
                    ) {
                        let combined = Arc::new(FilterNode::Or(
                            Box::new(left.clone()),
                            Box::new(right.clone()),
                        ));
                        drop_with_kind(a_bits, a_kind);
                        drop_with_kind(b_bits, b_kind);
                        let raw = Arc::into_raw(combined) as u64;
                        self.push_kinded(raw, NativeKind::Ptr(HeapKind::FilterExpr))?;
                    } else {
                        let r = kinded_truthy(a_bits, a_kind) || kinded_truthy(b_bits, b_kind);
                        drop_with_kind(a_bits, a_kind);
                        drop_with_kind(b_bits, b_kind);
                        self.push_kinded(r as u64, NativeKind::Bool)?;
                    }
                } else {
                    let r = kinded_truthy(a_bits, a_kind) || kinded_truthy(b_bits, b_kind);
                    self.push_kinded(r as u64, NativeKind::Bool)?;
                }
            }
            Not => {
                let (bits, kind) = self.pop_kinded()?;
                if kind_is_heap(kind) {
                    if let Some(node) = raw_helpers::extract_filter_expr(bits, kind) {
                        let combined = Arc::new(FilterNode::Not(Box::new(node.clone())));
                        drop_with_kind(bits, kind);
                        let raw = Arc::into_raw(combined) as u64;
                        self.push_kinded(raw, NativeKind::Ptr(HeapKind::FilterExpr))?;
                    } else {
                        let r = !kinded_truthy(bits, kind);
                        drop_with_kind(bits, kind);
                        self.push_kinded(r as u64, NativeKind::Bool)?;
                    }
                } else {
                    self.push_kinded(!kinded_truthy(bits, kind) as u64, NativeKind::Bool)?;
                }
            }
            _ => unreachable!(
                "exec_logical called with non-logical opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    /// Null coalescing operator: returns left if not null, otherwise right.
    /// Wave 6: null detection via the parallel kind track + zero-bits
    /// convention (Nullable* arms encode null as zero bits).
    pub(in crate::executor) fn op_null_coalesce(&mut self) -> Result<(), VMError> {
        let (right_bits, right_kind) = self.pop_kinded()?;
        let (left_bits, left_kind) = self.pop_kinded()?;

        let left_is_null = is_null_kinded(left_bits, left_kind);
        if left_is_null {
            // Discard left, push right.
            drop_with_kind(left_bits, left_kind);
            self.push_kinded(right_bits, right_kind)
        } else {
            // Discard right, push left.
            drop_with_kind(right_bits, right_kind);
            self.push_kinded(left_bits, left_kind)
        }
    }
}

/// Wave 6: null detection from raw bits + kind. Replaces the deleted
/// `ValueWord::is_none()`. Heap-bearing kinds are null when the pointer
/// bits are zero; nullable scalar arms encode null as zero bits.
#[inline]
fn is_null_kinded(bits: u64, kind: NativeKind) -> bool {
    match kind {
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
