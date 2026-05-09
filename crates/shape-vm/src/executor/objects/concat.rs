//! Dedicated concatenation opcodes (StringConcat, ArrayConcat).
//!
//! These replace the generic `OpCode::AddDynamic` overload for built-in heap
//! types whose operand types the compiler can prove statically. Operator
//! overloading on user-defined types still goes through `CallMethod`
//! (see Phase 2.5).
//!
//! Phase 1.B-vm Wave 6.5 substep-2 cluster D-obj-tail (playbook §10):
//!
//! - `op_string_concat` migrates to the kinded API. The operand kinds are
//!   compile-time known (`StringConcat` is only emitted when both operands
//!   are statically-proven `string` or `char`); the body dispatches on
//!   `NativeKind::String` (`Arc<String>` payload) and
//!   `NativeKind::Ptr(HeapKind::Char)` (codepoint payload, ADR-006 §2.7
//!   Char-as-inline-scalar shape per `stack_ops::op_push_const`).
//! - `op_array_concat` surfaces `NotImplemented(SURFACE)` per playbook §7.4
//!   DoD: the original v2 typed-array path uses `as_v2_typed_array` (which
//!   probes `ValueWord` internals — forbidden #1) plus `from_native_ptr`
//!   (deleted by ADR-005); the v1 generic path uses
//!   `vmarray_from_vec` + `from_array` (deprecated `Vec<ValueWord>` array
//!   shape). A correct kinded reimplementation needs the typed-Arc
//!   `Arc<TypedArrayData>` walk per `NativeKind::Ptr(HeapKind::TypedArray)`
//!   discriminator, paired with element-kind dispatch (i64/f64/i32/bool)
//!   to memcpy-concat into a freshly allocated `TypedArrayData` — that
//!   coordination with ADR-006 §2.3 / §2.4 typed-Arc constructors is out
//!   of D-obj-tail territory.

use crate::executor::VirtualMachine;
use crate::executor::vm_impl::stack::drop_with_kind;
use shape_value::heap_value::HeapKind;
use shape_value::{NativeKind, VMError};
use std::sync::Arc;

impl VirtualMachine {
    /// Concatenate two heap strings/chars, push the resulting string.
    ///
    /// Stack: `[a, b]` → `[a ++ b]`. Accepts any combination of
    /// `String + String`, `String + Char`, `Char + String`, `Char + Char`.
    /// All other operand combinations are a runtime type error (the compiler
    /// is supposed to only emit this opcode when both operands are
    /// statically proven to be `string` or `char`).
    #[inline]
    pub(in crate::executor) fn op_string_concat(&mut self) -> Result<(), VMError> {
        // ADR-006 §2.7.7 / Wave 6.5: pop_kinded transfers ownership for
        // each operand's `Arc<String>` strong-count share (when the kind
        // is heap-bearing). The handler must explicitly drop_with_kind
        // each consumed share before pushing the result.
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;

        // Borrow each operand's payload long enough to copy its bytes
        // into the result buffer. The `Arc<String>` reconstructions below
        // re-bump the strong count so the drop_with_kind at the end
        // releases the share that pop_kinded transferred to us.
        let result = match (read_string_or_char(a_bits, a_kind), read_string_or_char(b_bits, b_kind)) {
            (Some(a), Some(b)) => format!("{}{}", a, b),
            _ => {
                // Release the popped shares before erroring.
                drop_with_kind(a_bits, a_kind);
                drop_with_kind(b_bits, b_kind);
                return Err(VMError::TypeError {
                    expected: "string or char operands for StringConcat",
                    got: "non-string non-char",
                });
            }
        };

        // Release the input shares now that the bytes have been copied.
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);

        // Push the result as `Arc<String>` raw pointer + NativeKind::String.
        let arc: Arc<String> = Arc::new(result);
        let bits = Arc::into_raw(arc) as u64;
        self.push_kinded(bits, NativeKind::String)
    }

    /// Concatenate two arrays, push the resulting array.
    ///
    /// Stack: `[a, b]` → `[a ++ b]`.
    ///
    /// Surfaces `NotImplemented(SURFACE)` per playbook §7.4: the typed-Arc
    /// `Arc<TypedArrayData>` walk + element-kind dispatch for memcpy
    /// concat is out of D-obj-tail territory; depends on ADR-006 §2.3 /
    /// §2.4 typed-Arc constructor migration.
    #[inline]
    pub(in crate::executor) fn op_array_concat(&mut self) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "phase-2c — ArrayConcat: typed-Arc Arc<TypedArrayData> walk + \
             element-kind memcpy concat (ADR-006 §2.3 / §2.7.7); v1 generic \
             Vec<ValueWord> array path retired with ValueWord deletion"
                .to_string(),
        ))
    }
}

/// Read a `string` or `char` operand's payload as a borrow.
///
/// Returns:
/// - `Some(String)` for `NativeKind::String` (or
///   `NativeKind::Ptr(HeapKind::String)`) — reconstructs the `Arc<String>`,
///   clones the inner string, and lets the temporary `Arc` go (decrementing
///   the strong count we just synthesized via `Arc::from_raw` /
///   `Arc::increment_strong_count`).
/// - `Some(String)` of length 1 for `NativeKind::Ptr(HeapKind::Char)` —
///   the codepoint is inline in `bits` (per `stack_ops::op_push_const`'s
///   Char arm).
/// - `None` for any other kind (caller surfaces a TypeError).
///
/// The returned `String` is owned and independent of the operand bits;
/// the caller still owns the original strong-count share and must
/// release it via `drop_with_kind` after this function returns.
#[inline]
fn read_string_or_char(bits: u64, kind: NativeKind) -> Option<String> {
    match kind {
        NativeKind::String => unsafe {
            // Bump the strong count, reconstruct the Arc, copy the inner
            // String, then drop the temporary Arc (which decrements the
            // count we just bumped). The original share that pop_kinded
            // handed us stays live until the caller's drop_with_kind.
            Arc::increment_strong_count(bits as *const String);
            let arc: Arc<String> = Arc::from_raw(bits as *const String);
            let s = (*arc).clone();
            drop(arc);
            Some(s)
        },
        NativeKind::Ptr(HeapKind::String) => unsafe {
            Arc::increment_strong_count(bits as *const String);
            let arc: Arc<String> = Arc::from_raw(bits as *const String);
            let s = (*arc).clone();
            drop(arc);
            Some(s)
        },
        NativeKind::Ptr(HeapKind::Char) => {
            // Char is encoded as the codepoint in the low 32 bits per
            // stack_ops::op_push_const. ADR-006 §2.7.7 / playbook §3
            // Char arm: codepoint as u64, no Arc.
            char::from_u32(bits as u32).map(|c| c.to_string())
        }
        _ => None,
    }
}
