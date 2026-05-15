//! Array operations (ArrayPush, ArrayPushLocal, ArrayPop, SliceAccess)
//!
//! Handles array manipulation and slicing for v2-raw typed arrays.
//!
//! ## V3-S5 ckpt-5 consumer-cascade tier 3 surface (2026-05-15)
//!
//! Per V3-S5 ckpt-1..ckpt-4 cascade the `TypedArrayData` enum +
//! `TypedBuffer<T>` / `AlignedTypedBuffer` wrapper layer +
//! `HeapValue::TypedArray(Arc<TypedArrayData>)` outer arm +
//! `HeapKind::TypedArray = 8` ordinal were DELETED wholesale per
//! W12-typed-array-data-deletion audit §3.5 + §3.6 + §B + ADR-006
//! §2.7.24 Q25.A SUPERSEDED. The `NativeKind::Ptr(HeapKind::TypedArray)`
//! receiver shape is gone.
//!
//! The pre-ckpt-1 file had two carrier paths:
//!   - `Ptr(HeapKind::TypedArray)` Arc-boxed `Arc<TypedArrayData>` (DELETED)
//!   - `NativeKind::UInt64` v2-raw `*mut TypedArray<T>` (PRESERVED)
//!
//! The Arc-boxed path's helpers (`element_kind_of`, `push_into_typed_array`,
//! `pop_from_typed_array`, `slice_typed_array`) are DELETED. The
//! UInt64 v2-raw path is the canonical pattern per W12 audit §A.3 +
//! §3.1 scalar recipe and stays live.
//!
//! Refusal #1 binding: TypedArrayData resurrection under any rename
//! refused on sight.

use crate::bytecode::{Instruction, Operand};
use crate::executor::v2_handlers::v2_array_detect::{
    self, ELEM_TYPE_BOOL, ELEM_TYPE_CHAR, ELEM_TYPE_DECIMAL, ELEM_TYPE_F32, ELEM_TYPE_F64,
    ELEM_TYPE_I16, ELEM_TYPE_I32, ELEM_TYPE_I64, ELEM_TYPE_I8, ELEM_TYPE_STRING, ELEM_TYPE_U16,
    ELEM_TYPE_U32, ELEM_TYPE_U8,
    V2ElemType,
    V2TypedArrayView,
};
use crate::executor::vm_impl::stack::drop_with_kind;
use crate::executor::VirtualMachine;
use shape_value::v2::typed_array::TypedArray;
use shape_value::{NativeKind, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-5 surface-and-stop builder (for the deleted Ptr(HeapKind::
// TypedArray) Arc<TypedArrayData> receiver arm)
// ═══════════════════════════════════════════════════════════════════════════

#[cold]
#[inline(never)]
fn ckpt5_typed_array_surface(op: &'static str, kind: NativeKind) -> VMError {
    VMError::NotImplemented(format!(
        "{op}: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. \
         `Arc<TypedArrayData>` carrier + `HeapKind::TypedArray=8` ordinal \
         DELETED at V3-S5 ckpt-1..ckpt-4 per W12-typed-array-data-deletion \
         audit §3.5 + §3.6 + ADR-006 §2.7.24 Q25.A SUPERSEDED. UInt64 \
         v2-raw `*mut TypedArray<T>` receiver path remains live. Receiver \
         kind: {kind:?}. REFUSED ON SIGHT: TypedArrayData resurrection \
         under any rename (Refusal #1).",
        op = op,
        kind = kind,
    ))
}

impl VirtualMachine {
    pub(in crate::executor) fn op_array_push(&mut self) -> Result<(), VMError> {
        // Stack discipline: ArrayPush expects [array, value] with `value` at top.
        let (value_bits, value_kind) = self.pop_kinded()?;
        let (array_bits, array_kind) = self.pop_kinded()?;

        match array_kind {
            NativeKind::UInt64 => {
                // v2 typed-array carrier (raw `*mut TypedArray<T>` with
                // UInt64 kind). Detect → push_element → re-stash.
                match v2_array_detect::as_v2_typed_array(array_bits, array_kind) {
                    Some(view) => {
                        match v2_array_detect::push_element(
                            &view, value_bits, value_kind,
                        ) {
                            Ok(()) => {
                                self.push_kinded(array_bits, NativeKind::UInt64)
                            }
                            Err(msg) => {
                                drop_with_kind(value_bits, value_kind);
                                Err(VMError::TypeError {
                                    expected: "v2 typed-array element",
                                    got: msg,
                                })
                            }
                        }
                    }
                    None => {
                        drop_with_kind(value_bits, value_kind);
                        Err(VMError::NotImplemented(
                            "ArrayPush: UInt64 receiver did not resolve to a \
                             v2 typed-array pointer (HEAP_KIND_V2_TYPED_ARRAY \
                             header missing). ADR-006 §2.7.6 / §2.7.7."
                                .to_string(),
                        ))
                    }
                }
            }
            _ => {
                drop_with_kind(value_bits, value_kind);
                drop_with_kind(array_bits, array_kind);
                Err(ckpt5_typed_array_surface("ArrayPush", array_kind))
            }
        }
    }

    /// Push a value into an array stored in a local or module_binding
    /// variable slot, mutating in-place.
    pub(in crate::executor) fn op_array_push_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (value_bits, value_kind) = self.pop_kinded()?;

        let receiver_loc = match instruction.operand {
            Some(Operand::Local(idx)) => ReceiverLoc::Local(idx as usize),
            Some(Operand::ModuleBinding(idx)) => ReceiverLoc::ModuleBinding(idx as usize),
            _ => {
                drop_with_kind(value_bits, value_kind);
                return Err(VMError::InvalidOperand);
            }
        };

        let (_peek_bits, array_kind) = self.read_receiver_loc(&receiver_loc);

        match array_kind {
            NativeKind::UInt64 => {
                let (array_bits, _) = self.read_receiver_loc(&receiver_loc);
                match v2_array_detect::as_v2_typed_array(array_bits, array_kind) {
                    Some(view) => {
                        match v2_array_detect::push_element(
                            &view, value_bits, value_kind,
                        ) {
                            Ok(()) => Ok(()),
                            Err(msg) => {
                                drop_with_kind(value_bits, value_kind);
                                Err(VMError::TypeError {
                                    expected: "v2 typed-array element",
                                    got: msg,
                                })
                            }
                        }
                    }
                    None => {
                        drop_with_kind(value_bits, value_kind);
                        Err(VMError::NotImplemented(
                            "ArrayPushLocal: UInt64 slot did not resolve to a \
                             v2 typed-array pointer (HEAP_KIND_V2_TYPED_ARRAY \
                             header missing). ADR-006 §2.7.6 / §2.7.7."
                                .to_string(),
                        ))
                    }
                }
            }
            _ => {
                drop_with_kind(value_bits, value_kind);
                Err(ckpt5_typed_array_surface("ArrayPushLocal", array_kind))
            }
        }
    }

    /// Borrow the receiver `(bits, kind)` from the slot — no refcount
    /// change, slot retains ownership.
    fn read_receiver_loc(&self, loc: &ReceiverLoc) -> (u64, NativeKind) {
        match loc {
            ReceiverLoc::Local(idx) => {
                let bp = self.current_locals_base();
                self.stack_read_kinded_raw(bp + *idx)
            }
            ReceiverLoc::ModuleBinding(idx) => self.module_binding_read_kinded_raw(*idx),
        }
    }

    pub(in crate::executor) fn op_array_pop(&mut self) -> Result<(), VMError> {
        let (array_bits, array_kind) = self.pop_kinded()?;

        match array_kind {
            NativeKind::UInt64 => {
                match v2_array_detect::as_v2_typed_array(array_bits, array_kind) {
                    Some(view) => {
                        let result = v2_array_detect::pop_element(&view);
                        let _ = array_bits;
                        match result {
                            Some((val_bits, val_kind)) => {
                                self.push_kinded(val_bits, val_kind)
                            }
                            None => self.push_kinded(0u64, NativeKind::Bool),
                        }
                    }
                    None => Err(VMError::NotImplemented(
                        "ArrayPop: UInt64 receiver did not resolve to a v2 \
                         typed-array pointer (HEAP_KIND_V2_TYPED_ARRAY \
                         header missing). ADR-006 §2.7.6 / §2.7.7."
                            .to_string(),
                    )),
                }
            }
            _ => {
                drop_with_kind(array_bits, array_kind);
                Err(ckpt5_typed_array_surface("ArrayPop", array_kind))
            }
        }
    }

    pub(in crate::executor) fn op_slice_access(&mut self) -> Result<(), VMError> {
        // SliceAccess: [array, start, end] -> [slice]
        let (end_bits, end_kind) = self.pop_kinded()?;
        let (start_bits, start_kind) = self.pop_kinded()?;
        let (array_bits, array_kind) = self.pop_kinded()?;

        let start = match index_from_kinded(start_bits, start_kind) {
            Ok(i) => i,
            Err(e) => {
                drop_with_kind(end_bits, end_kind);
                drop_with_kind(start_bits, start_kind);
                drop_with_kind(array_bits, array_kind);
                return Err(e);
            }
        };
        let end = match index_from_kinded(end_bits, end_kind) {
            Ok(i) => i,
            Err(e) => {
                drop_with_kind(end_bits, end_kind);
                drop_with_kind(array_bits, array_kind);
                return Err(e);
            }
        };
        let _ = (start_bits, end_bits);

        match array_kind {
            NativeKind::UInt64 => {
                match v2_array_detect::as_v2_typed_array(array_bits, array_kind) {
                    Some(view) => {
                        let (s, e) = clamp_range(start, end, view.len as i64);
                        let new_ptr = slice_v2_typed_array(&view, s, e);
                        let _ = array_bits;
                        self.push_kinded(new_ptr as u64, NativeKind::UInt64)
                    }
                    None => Err(VMError::NotImplemented(
                        "SliceAccess: UInt64 receiver did not resolve to a \
                         v2 typed-array pointer (HEAP_KIND_V2_TYPED_ARRAY \
                         header missing). ADR-006 §2.7.6 / §2.7.7."
                            .to_string(),
                    )),
                }
            }
            NativeKind::String => {
                // String slicing — out of W17-array-typed-receiver
                // territory; surface (legacy `as_heap_ref` + `HeapValue::
                // String` dispatch was forbidden-pattern).
                drop_with_kind(array_bits, array_kind);
                Err(VMError::NotImplemented(
                    "SliceAccess: string receiver — needs dedicated op. \
                     ADR-006 §2.7.6 / §2.7.7."
                        .to_string(),
                ))
            }
            _ => {
                drop_with_kind(array_bits, array_kind);
                Err(ckpt5_typed_array_surface("SliceAccess", array_kind))
            }
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Receiver-location descriptor for `ArrayPushLocal`
// ───────────────────────────────────────────────────────────────────────────

enum ReceiverLoc {
    Local(usize),
    ModuleBinding(usize),
}

// ───────────────────────────────────────────────────────────────────────────
// V3-S5 ckpt-5 (2026-05-15): TypedArrayData-dependent helpers DELETED.
// `element_kind_of` / `push_into_typed_array` / `pop_from_typed_array` /
// `slice_typed_array` consumed `Arc<TypedArrayData>` (deleted at
// ckpt-1..ckpt-4); the four `op_array_*` consumers above surface-and-stop
// for the deleted Ptr(HeapKind::TypedArray) arm.
// ───────────────────────────────────────────────────────────────────────────

/// Clamp Python-style negative indices and bound them to `[0, len]`.
fn clamp_range(start: i64, end: i64, len: i64) -> (usize, usize) {
    let s = if start < 0 {
        (len + start).max(0)
    } else {
        start.min(len)
    };
    let e = if end < 0 {
        (len + end).max(0)
    } else {
        end.min(len)
    };
    (s as usize, e as usize)
}

/// Slice a v2 typed array `[s, e)` into a freshly-allocated
/// `TypedArray<T>` of the same element type. Returns the raw pointer
/// (slot carrier shape — `NativeKind::UInt64`).
fn slice_v2_typed_array(
    view: &V2TypedArrayView,
    s: usize,
    e: usize,
) -> *mut u8 {
    use crate::executor::v2_handlers::v2_array_detect::stamp_elem_type;
    let (s, e) = if s <= e { (s, e) } else { (s, s) };
    match view.elem_type {
        V2ElemType::F64 => unsafe {
            let src = view.ptr as *const TypedArray<f64>;
            let slice: &[f64] = if s < e {
                let data = (*src).data as *const f64;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<f64>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_F64);
            new_ptr as *mut u8
        },
        V2ElemType::I64 => unsafe {
            let src = view.ptr as *const TypedArray<i64>;
            let slice: &[i64] = if s < e {
                let data = (*src).data as *const i64;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<i64>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_I64);
            new_ptr as *mut u8
        },
        V2ElemType::I32 => unsafe {
            let src = view.ptr as *const TypedArray<i32>;
            let slice: &[i32] = if s < e {
                let data = (*src).data as *const i32;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<i32>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_I32);
            new_ptr as *mut u8
        },
        V2ElemType::Bool => unsafe {
            let src = view.ptr as *const TypedArray<u8>;
            let slice: &[u8] = if s < e {
                let data = (*src).data as *const u8;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<u8>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_BOOL);
            new_ptr as *mut u8
        },
        V2ElemType::I8 => unsafe {
            let src = view.ptr as *const TypedArray<i8>;
            let slice: &[i8] = if s < e {
                let data = (*src).data as *const i8;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<i8>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_I8);
            new_ptr as *mut u8
        },
        V2ElemType::U8 => unsafe {
            let src = view.ptr as *const TypedArray<u8>;
            let slice: &[u8] = if s < e {
                let data = (*src).data as *const u8;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<u8>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_U8);
            new_ptr as *mut u8
        },
        V2ElemType::I16 => unsafe {
            let src = view.ptr as *const TypedArray<i16>;
            let slice: &[i16] = if s < e {
                let data = (*src).data as *const i16;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<i16>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_I16);
            new_ptr as *mut u8
        },
        V2ElemType::U16 => unsafe {
            let src = view.ptr as *const TypedArray<u16>;
            let slice: &[u16] = if s < e {
                let data = (*src).data as *const u16;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<u16>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_U16);
            new_ptr as *mut u8
        },
        V2ElemType::U32 => unsafe {
            let src = view.ptr as *const TypedArray<u32>;
            let slice: &[u32] = if s < e {
                let data = (*src).data as *const u32;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<u32>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_U32);
            new_ptr as *mut u8
        },
        V2ElemType::F32 => unsafe {
            let src = view.ptr as *const TypedArray<f32>;
            let slice: &[f32] = if s < e {
                let data = (*src).data as *const f32;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<f32>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_F32);
            new_ptr as *mut u8
        },
        V2ElemType::Char => unsafe {
            let src = view.ptr as *const TypedArray<char>;
            let slice: &[char] = if s < e {
                let data = (*src).data as *const char;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<char>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_CHAR);
            new_ptr as *mut u8
        },
        V2ElemType::String => unsafe {
            use shape_value::v2::refcount::v2_retain;
            use shape_value::v2::string_obj::StringObj;
            let src = view.ptr as *const TypedArray<*const StringObj>;
            let count = e.saturating_sub(s);
            let new_ptr = TypedArray::<*const StringObj>::with_capacity(count as u32);
            if count > 0 {
                let src_data = (*src).data;
                let dst_data = (*new_ptr).data;
                for i in 0..count {
                    let elem = *src_data.add(s + i);
                    v2_retain(&(*elem).header);
                    *dst_data.add(i) = elem;
                }
                (*new_ptr).len = count as u32;
            }
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_STRING);
            new_ptr as *mut u8
        },
        V2ElemType::Decimal => unsafe {
            use shape_value::v2::decimal_obj::DecimalObj;
            use shape_value::v2::refcount::v2_retain;
            let src = view.ptr as *const TypedArray<*const DecimalObj>;
            let count = e.saturating_sub(s);
            let new_ptr = TypedArray::<*const DecimalObj>::with_capacity(count as u32);
            if count > 0 {
                let src_data = (*src).data;
                let dst_data = (*new_ptr).data;
                for i in 0..count {
                    let elem = *src_data.add(s + i);
                    v2_retain(&(*elem).header);
                    *dst_data.add(i) = elem;
                }
                (*new_ptr).len = count as u32;
            }
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_DECIMAL);
            new_ptr as *mut u8
        },
    }
}

/// True if `kind` is one of the integer-family `NativeKind`s.
#[inline]
fn is_int_kind(kind: NativeKind) -> bool {
    matches!(
        kind,
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
    )
}

/// Read a slice index from a kinded slot.
#[inline]
fn index_from_kinded(bits: u64, kind: NativeKind) -> Result<i64, VMError> {
    if is_int_kind(kind) {
        Ok(bits as i64)
    } else {
        Err(VMError::TypeError {
            expected: "integer index",
            got: "non-integer kind",
        })
    }
}
