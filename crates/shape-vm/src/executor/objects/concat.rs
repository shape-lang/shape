//! Dedicated concatenation opcodes (StringConcat, ArrayConcat).
//!
//! These replace the generic `OpCode::AddDynamic` overload for built-in heap
//! types whose operand types the compiler can prove statically. Operator
//! overloading on user-defined types still goes through `CallMethod`
//! (see Phase 2.5).
//!
//! ## V3-S5 ckpt-3 consumer-cascade tier 2 surface (2026-05-15)
//!
//! Per V3-S5 ckpt-1 close (commit `aac8495e`, 2026-05-15), the
//! `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
//! `typed_array_structural_eq` fn were DELETED at
//! `crates/shape-value/src/heap_value.rs` per W12-typed-array-data-deletion
//! audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. This file's previous
//! consumer-shape — `op_array_concat` recovering two `Arc<TypedArrayData>`
//! via `Arc::<TypedArrayData>::from_raw` and dispatching through
//! `concat_typed_arrays` over per-variant pair-arms (`TypedArrayData::I64 /
//! F64 / Bool / I8 / I16 / I32 / U8 / U16 / U32 / U64 / F32 / String /
//! Decimal / BigInt / Char / TypedObject`) — cascade-breaks here as the
//! deletion's consumer cascade tier 2.
//!
//! `op_array_concat` body is replaced with structured surface-and-stop
//! returning `VMError::NotImplemented`. The cross-variant TypeError
//! discrimination via `type_pair_static_str` is DELETED — it produced
//! `&'static str` from `TypedArrayData::type_name()` which is gone.
//! `concat_typed_arrays` is DELETED.
//!
//! PRESERVED:
//! - `op_string_concat` — no `TypedArrayData` dependency; operates on
//!   `Arc<String>` / `HeapKind::String` / `HeapKind::Char` arms only.
//! - `read_string_or_char` — no `TypedArrayData` dependency; consumed by
//!   `op_string_concat`.
//!
//! ## Cascade migration target (post-ckpt-6 STRICT close)
//!
//! Per W12-typed-array-data-deletion audit §A.3 + §2.1 scalar recipe +
//! §2.2 heap-element variants, every previous `TypedArrayData::X(buf)`
//! pair-arm in `concat_typed_arrays` migrates to the v2-raw `TypedArray<T>`
//! flat-struct carrier with per-T `data.extend_from_slice()` append. Both
//! operands stay typed-Arcs at the stack level (the pop_kinded contract is
//! unchanged); the per-T body comes from monomorphizing across the v2-raw
//! per-T allocator post-ckpt-6.
//!
//! Bodies REFUSED ON SIGHT under Refusal #1 (resurrection under rename
//! per ckpt-1 close-marker at `heap_value.rs:3956`).

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
    ///
    /// Preserved through V3-S5 ckpt-3 because the body has no
    /// `TypedArrayData` dependency.
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
    /// V3-S5 ckpt-3 surface-and-stop. The previous body recovered two
    /// `Arc<TypedArrayData>` via `Arc::<TypedArrayData>::from_raw` and
    /// dispatched through 16 per-variant pair-arms in `concat_typed_arrays`;
    /// the type is gone (ckpt-1 deletion). Post-ckpt-6 the body becomes a
    /// per-T v2-raw `TypedArray<T>` extend dispatch.
    ///
    /// Releases both operand shares before erroring to preserve refcount
    /// discipline per ADR-006 §2.7.7.
    #[inline]
    pub(in crate::executor) fn op_array_concat(&mut self) -> Result<(), VMError> {
        // Pop b then a (LIFO).
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;

        let kinds_ok = matches!(
            (a_kind, b_kind),
            (
                NativeKind::Ptr(HeapKind::TypedArray),
                NativeKind::Ptr(HeapKind::TypedArray),
            )
        );

        // Always release the popped shares — surface-and-stop emits an
        // error regardless of operand-kind validity, but the refcount
        // discipline is preserved per ADR-006 §2.7.7.
        drop_with_kind(b_bits, b_kind);
        drop_with_kind(a_bits, a_kind);

        if !kinds_ok {
            return Err(VMError::TypeError {
                expected: "two TypedArray operands for ArrayConcat",
                got: "non-TypedArray kind",
            });
        }
        Err(ckpt3_surface("ArrayConcat", a_kind, b_kind))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-3 surface-and-stop builder
// ═══════════════════════════════════════════════════════════════════════════

/// Surface-and-stop body for `op_array_concat`.
#[cold]
#[inline(never)]
fn ckpt3_surface(op: &'static str, a_kind: NativeKind, b_kind: NativeKind) -> VMError {
    VMError::NotImplemented(format!(
        "{op}: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface. \
         `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per W12-\
         typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A \
         SUPERSEDED. The previous `Arc<TypedArrayData>` operand-recovery \
         + per-variant pair-arm dispatch path (~32 references across \
         `concat_typed_arrays` 16 pair-arms) cascade-broke at the enum \
         deletion site (`crates/shape-value/src/heap_value.rs:3944`). \
         Post-deletion target is the v2-raw `TypedArray<T>` flat-struct \
         carrier per audit §1.2 + §A.3 + §3.1 scalar recipe + §2.2 \
         heap-element variants — per-T `data.extend_from_slice()` append; \
         landing across ckpt-3 (this file plus array_ops/typed_array_methods/\
         iterator_methods/array_sort/property_access/array_query) + ckpt-4 \
         (Buf<T> / HeapValue::TypedArray arm / HeapKind::TypedArray \
         ordinal) + ckpt-5 (wire/json/marshal + 4-table lockstep) + ckpt-6 \
         (JIT FFI). Operand kinds: a={a_kind:?}, b={b_kind:?}. UNREACHABLE \
         until ckpt-6 STRICT close. REFUSED ON SIGHT: TypedArrayData \
         resurrection under any rename (Refusal #1, W12 audit §7).",
        op = op,
        a_kind = a_kind,
        b_kind = b_kind,
    ))
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
///
/// Preserved through V3-S5 ckpt-3 because the body has no `TypedArrayData`
/// dependency.
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
