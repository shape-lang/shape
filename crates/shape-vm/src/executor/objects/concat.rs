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
//! - `op_array_concat` (Wave-δ MR-string-misc): typed-Arc
//!   `Arc<TypedArrayData>` walk + same-variant element-kind concat.
//!   Cross-variant operand combinations are a runtime type error
//!   (CLAUDE.md "No runtime coercion" — same-variant only matches
//!   `op_string_concat`'s "operand kinds were proven by the compiler"
//!   contract). The typed-Arc constructor pattern mirrors the slice
//!   pattern in `objects/array_operations.rs::slice_typed_array`
//!   (which builds a fresh `Arc<TypedArrayData>` per source variant).

use crate::executor::VirtualMachine;
use crate::executor::vm_impl::stack::drop_with_kind;
use shape_value::heap_value::{HeapKind, TypedArrayData};
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
    /// Both operands must be `Ptr(HeapKind::TypedArray)` per the §2.7.7
    /// stack ABI; cross-variant element kinds (e.g. `Vec<int>` ++
    /// `Vec<number>`) are a runtime type error per CLAUDE.md "No runtime
    /// coercion" — the compiler is responsible for emitting matching
    /// variants (or surfacing a type error at compile time).
    #[inline]
    pub(in crate::executor) fn op_array_concat(&mut self) -> Result<(), VMError> {
        // Pop b then a (LIFO).
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;

        match (a_kind, b_kind) {
            (
                NativeKind::Ptr(HeapKind::TypedArray),
                NativeKind::Ptr(HeapKind::TypedArray),
            ) => {}
            _ => {
                drop_with_kind(b_bits, b_kind);
                drop_with_kind(a_bits, a_kind);
                return Err(VMError::TypeError {
                    expected: "two TypedArray operands for ArrayConcat",
                    got: "non-TypedArray kind",
                });
            }
        }

        // Reconstruct the typed Arcs. `from_raw` takes ownership of each
        // pop_kinded share; the read-only walks below borrow `&**arc` so
        // the shares stay alive until the explicit drops at end.
        let a = unsafe {
            Arc::<TypedArrayData>::from_raw(a_bits as *const TypedArrayData)
        };
        let b = unsafe {
            Arc::<TypedArrayData>::from_raw(b_bits as *const TypedArrayData)
        };

        let result = concat_typed_arrays(&a, &b);

        // Retire the source shares now that the result has been built.
        drop(a);
        drop(b);

        match result {
            Ok(arc) => {
                let bits = Arc::into_raw(arc) as u64;
                self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray))
            }
            Err(e) => Err(e),
        }
    }
}

/// Concatenate two `Arc<TypedArrayData>` values into a fresh
/// `Arc<TypedArrayData>` of the same variant. Cross-variant
/// (heterogeneous element kinds) is a `TypeError` — the compiler is
/// expected to prove element-kind compatibility at emit time.
///
/// Mirrors `slice_typed_array` in `array_operations.rs` for the
/// per-variant constructor pattern.
fn concat_typed_arrays(
    a: &Arc<TypedArrayData>,
    b: &Arc<TypedArrayData>,
) -> Result<Arc<TypedArrayData>, VMError> {
    match (&**a, &**b) {
        (TypedArrayData::I64(la), TypedArrayData::I64(lb)) => {
            let mut data: Vec<i64> = Vec::with_capacity(la.data.len() + lb.data.len());
            data.extend_from_slice(&la.data);
            data.extend_from_slice(&lb.data);
            let buf = shape_value::typed_buffer::TypedBuffer::from_vec(data);
            Ok(Arc::new(TypedArrayData::I64(Arc::new(buf))))
        }
        (TypedArrayData::F64(la), TypedArrayData::F64(lb)) => {
            let mut data: Vec<f64> =
                Vec::with_capacity(la.data.len() + lb.data.len());
            data.extend_from_slice(&la.data);
            data.extend_from_slice(&lb.data);
            let aligned = shape_value::aligned_vec::AlignedVec::<f64>::from_vec(data);
            let buf =
                shape_value::typed_buffer::AlignedTypedBuffer::from_aligned(aligned);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(buf))))
        }
        (TypedArrayData::Bool(la), TypedArrayData::Bool(lb)) => {
            let mut data: Vec<u8> = Vec::with_capacity(la.data.len() + lb.data.len());
            data.extend_from_slice(&la.data);
            data.extend_from_slice(&lb.data);
            let buf = shape_value::typed_buffer::TypedBuffer::from_vec(data);
            Ok(Arc::new(TypedArrayData::Bool(Arc::new(buf))))
        }
        (TypedArrayData::I8(la), TypedArrayData::I8(lb)) => {
            let mut data: Vec<i8> = Vec::with_capacity(la.data.len() + lb.data.len());
            data.extend_from_slice(&la.data);
            data.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::I8(Arc::new(
                shape_value::typed_buffer::TypedBuffer::from_vec(data),
            ))))
        }
        (TypedArrayData::I16(la), TypedArrayData::I16(lb)) => {
            let mut data: Vec<i16> = Vec::with_capacity(la.data.len() + lb.data.len());
            data.extend_from_slice(&la.data);
            data.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::I16(Arc::new(
                shape_value::typed_buffer::TypedBuffer::from_vec(data),
            ))))
        }
        (TypedArrayData::I32(la), TypedArrayData::I32(lb)) => {
            let mut data: Vec<i32> = Vec::with_capacity(la.data.len() + lb.data.len());
            data.extend_from_slice(&la.data);
            data.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::I32(Arc::new(
                shape_value::typed_buffer::TypedBuffer::from_vec(data),
            ))))
        }
        (TypedArrayData::U8(la), TypedArrayData::U8(lb)) => {
            let mut data: Vec<u8> = Vec::with_capacity(la.data.len() + lb.data.len());
            data.extend_from_slice(&la.data);
            data.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::U8(Arc::new(
                shape_value::typed_buffer::TypedBuffer::from_vec(data),
            ))))
        }
        (TypedArrayData::U16(la), TypedArrayData::U16(lb)) => {
            let mut data: Vec<u16> = Vec::with_capacity(la.data.len() + lb.data.len());
            data.extend_from_slice(&la.data);
            data.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::U16(Arc::new(
                shape_value::typed_buffer::TypedBuffer::from_vec(data),
            ))))
        }
        (TypedArrayData::U32(la), TypedArrayData::U32(lb)) => {
            let mut data: Vec<u32> = Vec::with_capacity(la.data.len() + lb.data.len());
            data.extend_from_slice(&la.data);
            data.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::U32(Arc::new(
                shape_value::typed_buffer::TypedBuffer::from_vec(data),
            ))))
        }
        (TypedArrayData::U64(la), TypedArrayData::U64(lb)) => {
            let mut data: Vec<u64> = Vec::with_capacity(la.data.len() + lb.data.len());
            data.extend_from_slice(&la.data);
            data.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::U64(Arc::new(
                shape_value::typed_buffer::TypedBuffer::from_vec(data),
            ))))
        }
        (TypedArrayData::F32(la), TypedArrayData::F32(lb)) => {
            let mut data: Vec<f32> = Vec::with_capacity(la.data.len() + lb.data.len());
            data.extend_from_slice(&la.data);
            data.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::F32(Arc::new(
                shape_value::typed_buffer::TypedBuffer::from_vec(data),
            ))))
        }
        (TypedArrayData::String(la), TypedArrayData::String(lb)) => {
            let mut data: Vec<Arc<String>> =
                Vec::with_capacity(la.data.len() + lb.data.len());
            for s in la.data.iter() {
                data.push(Arc::clone(s));
            }
            for s in lb.data.iter() {
                data.push(Arc::clone(s));
            }
            Ok(Arc::new(TypedArrayData::String(Arc::new(
                shape_value::typed_buffer::TypedBuffer::from_vec(data),
            ))))
        }
        (TypedArrayData::HeapValue(la), TypedArrayData::HeapValue(lb)) => {
            let mut data: Vec<Arc<shape_value::heap_value::HeapValue>> =
                Vec::with_capacity(la.data.len() + lb.data.len());
            for h in la.data.iter() {
                data.push(Arc::clone(h));
            }
            for h in lb.data.iter() {
                data.push(Arc::clone(h));
            }
            Ok(Arc::new(TypedArrayData::HeapValue(Arc::new(
                shape_value::typed_buffer::TypedBuffer::from_vec(data),
            ))))
        }
        // FloatSlice + FloatSlice: materialize into an owned F64 array
        // (matches the slice operator's materialize-not-view semantics
        // in `array_operations.rs::slice_typed_array`).
        (
            TypedArrayData::FloatSlice {
                parent: pa,
                offset: oa,
                len: la,
            },
            TypedArrayData::FloatSlice {
                parent: pb,
                offset: ob,
                len: lb,
            },
        ) => {
            let mut data: Vec<f64> = Vec::with_capacity(*la as usize + *lb as usize);
            let oa = *oa as usize;
            let la = *la as usize;
            let ob = *ob as usize;
            let lb = *lb as usize;
            data.extend_from_slice(&pa.data[oa..oa + la]);
            data.extend_from_slice(&pb.data[ob..ob + lb]);
            let aligned = shape_value::aligned_vec::AlignedVec::<f64>::from_vec(data);
            let buf =
                shape_value::typed_buffer::AlignedTypedBuffer::from_aligned(aligned);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(buf))))
        }
        // Cross-FloatSlice <-> F64 cases: materialize the slice and
        // concat through the F64 path. Avoids a Phase-2c surface for
        // a near-identical-shape concat — both arms are f64 storage.
        (
            TypedArrayData::FloatSlice {
                parent: pa,
                offset: oa,
                len: la,
            },
            TypedArrayData::F64(lb),
        ) => {
            let mut data: Vec<f64> = Vec::with_capacity(*la as usize + lb.data.len());
            let oa = *oa as usize;
            let la = *la as usize;
            data.extend_from_slice(&pa.data[oa..oa + la]);
            data.extend_from_slice(&lb.data);
            let aligned = shape_value::aligned_vec::AlignedVec::<f64>::from_vec(data);
            let buf =
                shape_value::typed_buffer::AlignedTypedBuffer::from_aligned(aligned);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(buf))))
        }
        (
            TypedArrayData::F64(la),
            TypedArrayData::FloatSlice {
                parent: pb,
                offset: ob,
                len: lb,
            },
        ) => {
            let mut data: Vec<f64> = Vec::with_capacity(la.data.len() + *lb as usize);
            let ob = *ob as usize;
            let lb = *lb as usize;
            data.extend_from_slice(&la.data);
            data.extend_from_slice(&pb.data[ob..ob + lb]);
            let aligned = shape_value::aligned_vec::AlignedVec::<f64>::from_vec(data);
            let buf =
                shape_value::typed_buffer::AlignedTypedBuffer::from_aligned(aligned);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(buf))))
        }
        // Matrix concat is undefined (the slice operator surfaces too;
        // matrix is a 2D view, not a 1D Vec).
        (TypedArrayData::Matrix(_), _) | (_, TypedArrayData::Matrix(_)) => {
            Err(VMError::NotImplemented(format!(
                "ArrayConcat: Matrix variant — Phase-2c reentry. Matrix \
                 is a 2D view, not a 1D Vec; concat semantics undefined \
                 (mirrors slice_typed_array in array_operations.rs)."
            )))
        }
        // Cross-variant: mismatched element kinds. Compiler should
        // have caught this at type-check; surface as a runtime type
        // error per CLAUDE.md "No runtime coercion".
        (lhs, rhs) => Err(VMError::TypeError {
            expected: "matching TypedArrayData element variants for ArrayConcat",
            got: type_pair_static_str(lhs.type_name(), rhs.type_name()),
        }),
    }
}

/// Map (lhs_name, rhs_name) into a static `&'static str` for the
/// TypeError's `got` field. The variant set is finite; unknown pairs
/// fall through to a generic "mismatched variants" tag.
#[inline]
fn type_pair_static_str(lhs: &'static str, rhs: &'static str) -> &'static str {
    // Common mismatches we want named explicitly for diagnostics.
    match (lhs, rhs) {
        ("Vec<int>", "Vec<number>") => "Vec<int> ++ Vec<number>",
        ("Vec<number>", "Vec<int>") => "Vec<number> ++ Vec<int>",
        ("Vec<int>", "Vec<bool>") => "Vec<int> ++ Vec<bool>",
        ("Vec<bool>", "Vec<int>") => "Vec<bool> ++ Vec<int>",
        ("Vec<string>", "Vec<int>") => "Vec<string> ++ Vec<int>",
        ("Vec<int>", "Vec<string>") => "Vec<int> ++ Vec<string>",
        _ => "mismatched TypedArray element variants",
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
