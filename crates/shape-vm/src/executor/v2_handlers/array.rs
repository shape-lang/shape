//! VM executor handlers for v2 typed array opcodes.
//!
//! These handlers operate on `TypedArray<T>` raw pointers (`*mut TypedArray<T>`),
//! a v2-raw HeapHeader-equipped flat struct (24-byte `repr(C)`, refcount at
//! offset 0). Pointer bits flow through the kinded API as
//! `NativeKind::Ptr(HeapKind::TypedArray)` — the kind track itself is the
//! carrier discriminator, separating a `*mut TypedArray<T>` pointer from a
//! genuine scalar `u64`. Element kinds:
//!   F64  -> `NativeKind::Float64`
//!   I64  -> `NativeKind::Int64`
//!   I32  -> `NativeKind::Int32`
//!   Bool -> `NativeKind::Bool`
//!
//! ## r5c-2-β-CKPT-C u64-carrier-disambiguation (2026-05-20)
//!
//! The `NewTypedArray*` producers below previously stamped the carrier with
//! the bare `NativeKind::UInt64` kind — the SAME kind that labels a genuine
//! scalar `u64`. That overload made `as_v2_typed_array(bits, kind)`
//! dereference an arbitrary scalar `u64` value (e.g. `u64::MAX`) as a
//! `*const HeapHeader`, SIGSEGV-ing on `print(x)` for `let x: u64 = ...`.
//! The fix migrates the v2-typed-array POINTER carrier to
//! `NativeKind::Ptr(HeapKind::TypedArray)` (ordinal 8, already extant),
//! completing the Family-α struct-field / closure-capture migration. The
//! kind track now discriminates: `Ptr(HeapKind::TypedArray)` is the array
//! carrier, `UInt64` is a scalar. `clone_with_kind` / `drop_with_kind`
//! route the carrier through `retain_v2_typed_array` /
//! `release_v2_typed_array` (the array starts at refcount 1 from
//! `with_capacity`'s `HeapHeader::new`), giving the direct carrier the same
//! RAII discipline as the refcounted struct-field / closure-capture carrier.
//!
//! ADR-006 §2.3 / §2.7.7 / Wave 6.5 cluster C.
//!
//! ## W16.2-J.2 macro-generated per-kind arms (2026-05-22)
//!
//! Per `docs/cluster-audits/v0.3-w16-2-j-audit.md` §2.C + §6.A: the
//! per-kind opcode arms (TypedArrayKind variants × 4 ops:
//! New/Get/Push/Set) are macro-generated via `macro_rules!` rather than
//! hand-written. A single `define_exec_v2_typed_array!` macro takes three
//! repetition groups (scalar rows, then char rows, then heap rows) and
//! emits the entire `impl VirtualMachine { fn exec_v2_typed_array(…) … }`
//! in one non-recursive expansion. The whole-impl shape sidesteps two
//! `macro_rules!` constraints: (a) macros in match-arm position must
//! produce all arms of the match in a single expansion (no chained
//! macro-arm-producers), and (b) TT-muncher recursion blows the
//! recursion limit because each step re-parses the accumulator
//! quadratically.
//!
//! Three row shapes cover the matrix:
//!
//! - `scalar` rows  — `Copy + Sized` element types with explicit
//!   bit↔value encode/decode expressions: F64, I64, I32, Bool, I8, U8,
//!   I16, U16, U32, F32. Same shape as the original hand-written arms;
//!   the macro just unrolls the four-op template per kind.
//!
//! - `char` rows    — Char element, push/set go through
//!   `char::from_u32` (rejects surrogates + out-of-range codepoints)
//!   and surface VMError::RuntimeError on invalid bits.
//!
//! - `heap` rows    — heap-element kinds with per-element refcount
//!   discipline (String/Decimal/TypedObject). Element-read retains the
//!   per-element HeapHeader before pushing; element-write/push transfers
//!   the caller's refcount share to the array; element-set additionally
//!   releases the prior element's share.
//!
//! ADR-006 §2.7.5 stamp-at-compile-time discipline preserved: each arm
//! stamps the element-type metadata at the allocation/push/get/set
//! producer site; the kind is encoded in the opcode name (and the
//! associated macro row), never decoded at runtime. Zero-tag runtime
//! invariant preserved — no `is_tagged()`, no `tag_bits`, no `ValueWord`
//! shim, no dynamic fallback.
//!
//! Semantically equivalent to the pre-macro hand-written arms (same
//! pop/push order, same refcount discipline, same error surfaces).
//! No external proc-macro crate (e.g. `paste`) is used — the OpCode
//! variant names are passed explicitly per macro row so concatenation
//! is never required.

use crate::bytecode::{Instruction, OpCode, Operand};
use crate::executor::vm_impl::stack::drop_with_kind;
use shape_value::heap_value::{TraitObjectStorage, TypedObjectStorage};
use shape_value::v2::decimal_obj::DecimalObj;
use shape_value::v2::heap_element::HeapElement;
use shape_value::v2::refcount::v2_retain;
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::{TypedArray, TypedArrayElem};
use shape_value::{HeapKind, NativeKind, VMError};

use super::super::VirtualMachine;
use super::v2_array_detect::{
    ELEM_TYPE_BOOL, ELEM_TYPE_CHAR, ELEM_TYPE_DECIMAL, ELEM_TYPE_F32, ELEM_TYPE_F64, ELEM_TYPE_I8,
    ELEM_TYPE_I16, ELEM_TYPE_I32, ELEM_TYPE_I64, ELEM_TYPE_STRING, ELEM_TYPE_TRAIT_OBJECT,
    ELEM_TYPE_TYPED_ARRAY, ELEM_TYPE_TYPED_OBJECT, ELEM_TYPE_U8, ELEM_TYPE_U16, ELEM_TYPE_U32,
    stamp_elem_type,
};

// ─────────────────────────────────────────────────────────────────────────
// W16.2-J.2 macro — per-kind typed-array opcode arms (audit §2.C / §6.A)
// ─────────────────────────────────────────────────────────────────────────
//
// The macro has a single non-recursive arm that accepts three repetition
// groups in fixed order:
//   1. scalar rows  — emitted first in the outer match
//   2. char rows    — emitted next
//   3. heap rows    — emitted last
//
// All three groups expand inside the same `match` body, producing the
// 56 per-kind arms (14 kinds × 4 ops). Non-repetition opcodes
// (`NewStringV2`, `NewDecimalV2`, `TypedArrayLen`) live in the macro
// body's trailer.
//
// The opcode name encodes the kind statically (ADR-006 §2.7.5 producer-
// side stamp); the kind is NEVER decoded at runtime. NO tagged dispatch,
// NO ValueWord, NO dynamic fallback. ALL arms transfer the array
// pointer through `pop_kinded` / `push_kinded` with
// `NativeKind::Ptr(HeapKind::TypedArray)`, and route the array's RAII
// through `drop_with_kind`.

macro_rules! define_exec_v2_typed_array {
    (
        scalar_rows: [
            $(
                {
                    ops: $s_new:ident / $s_get:ident / $s_push:ident / $s_set:ident,
                    storage: $s_storage:ty,
                    elem_tag: $s_tag:expr,
                    result_kind: $s_kind:expr,
                    decode( $s_vb:ident ) => $s_decode:expr,
                    encode( $s_v:ident : $s_vt:ty ) => $s_encode:expr,
                }
            )+
        ],
        char_rows: [
            $(
                {
                    ops: $c_new:ident / $c_get:ident / $c_push:ident / $c_set:ident,
                    elem_tag: $c_tag:expr,
                    result_kind: $c_kind:expr,
                    push_label: $c_push_label:expr,
                    set_label: $c_set_label:expr,
                }
            )+
        ],
        heap_rows: [
            $(
                {
                    ops: $h_new:ident / $h_get:ident / $h_push:ident / $h_set:ident,
                    heap_obj: $h_obj:ty,
                    elem_kind: $h_kind:expr,
                    elem_tag: $h_tag:expr,
                    err_label: $h_err:expr,
                }
            )+
        ],
    ) => {
        impl VirtualMachine {
            /// Execute a v2 typed array opcode.
            pub(crate) fn exec_v2_typed_array(
                &mut self,
                instruction: &Instruction,
            ) -> Result<(), VMError> {
                match instruction.opcode {
                    // ─── W16.2-J.2 macro-generated per-kind arms ───────
                    //
                    // 14 kinds × 4 ops (New / Get / Push / Set) = 56
                    // arms total:
                    //   - 10 scalar  rows: F64, I64, I32, Bool, I8, U8,
                    //                      I16, U16, U32, F32 (40 arms)
                    //   -  1 char    row : Char (4 arms)
                    //   -  3 heap    rows: String, Decimal, TypedObject
                    //                      (12 arms)
                    //
                    // Sign-extension (I8/I16/I32) and zero-extension
                    // (U8/U16/U32) preserve the value's semantic when the
                    // i64 slot is later decoded by `decode_i64` per
                    // kind. F32 bits ride the low 32 bits of the slot;
                    // Bool collapses any nonzero source-bit to 1. Char
                    // goes through `char::from_u32` on push/set (rejects
                    // surrogates + out-of-range codepoints). Heap-
                    // element kinds (String/Decimal/TypedObject) carry
                    // per-element refcount discipline (retain on Get,
                    // transfer on Push, release-prior on Set).
                    //
                    // U64 typed-array opcode handlers intentionally NOT
                    // minted — see opcode_defs.rs comment block. The
                    // S1.5 sub-cluster re-mints
                    // OpCode::{New,Get,Push,Set}TypedArrayU64 + their
                    // handler bodies once the §2.7.7/Q9 NativeKind
                    // discriminator for "pointer to TypedArray<T>" vs
                    // "scalar u64" lands.
                    $(
                        OpCode::$s_new => {
                            let cap = match instruction.operand {
                                Some(Operand::Count(n)) => n as u32,
                                _ => 0,
                            };
                            let ptr = TypedArray::<$s_storage>::with_capacity(cap);
                            // ADR-006 §2.7.5 producer-side stamp.
                            unsafe { stamp_elem_type(ptr as *mut u8, $s_tag) };
                            self.push_kinded(
                                ptr as usize as u64,
                                NativeKind::Ptr(HeapKind::TypedArray),
                            )?;
                            Ok(())
                        }
                        OpCode::$s_get => {
                            let (idx_bits, _idx_kind) = self.pop_kinded()?;
                            let index = idx_bits as i64 as u32;
                            let (arr_bits, arr_kind) = self.pop_kinded()?;
                            let arr = arr_bits as usize as *const TypedArray<$s_storage>;
                            let len = unsafe { TypedArray::len(arr) };
                            let $s_v: $s_vt = unsafe {
                                TypedArray::get(arr, index).ok_or(
                                    VMError::IndexOutOfBounds {
                                        index: index as i32,
                                        length: len as usize,
                                    },
                                )?
                            };
                            drop_with_kind(arr_bits, arr_kind);
                            // ADR-006 §2.7.5: result kind statically
                            // pinned by the opcode name.
                            self.push_kinded($s_encode, $s_kind)?;
                            Ok(())
                        }
                        OpCode::$s_push => {
                            let ($s_vb, _vk) = self.pop_kinded()?;
                            let val: $s_storage = $s_decode;
                            let (arr_bits, arr_kind) = self.pop_kinded()?;
                            let arr = arr_bits as usize as *mut TypedArray<$s_storage>;
                            unsafe { TypedArray::push(arr, val); }
                            drop_with_kind(arr_bits, arr_kind);
                            Ok(())
                        }
                        OpCode::$s_set => {
                            let ($s_vb, _vk) = self.pop_kinded()?;
                            let val: $s_storage = $s_decode;
                            let (idx_bits, _ik) = self.pop_kinded()?;
                            let index = idx_bits as i64 as u32;
                            let (arr_bits, arr_kind) = self.pop_kinded()?;
                            let arr = arr_bits as usize as *mut TypedArray<$s_storage>;
                            unsafe { TypedArray::set(arr, index, val); }
                            drop_with_kind(arr_bits, arr_kind);
                            Ok(())
                        }
                    )+

                    $(
                        OpCode::$c_new => {
                            let cap = match instruction.operand {
                                Some(Operand::Count(n)) => n as u32,
                                _ => 0,
                            };
                            let ptr = TypedArray::<char>::with_capacity(cap);
                            unsafe { stamp_elem_type(ptr as *mut u8, $c_tag) };
                            self.push_kinded(
                                ptr as usize as u64,
                                NativeKind::Ptr(HeapKind::TypedArray),
                            )?;
                            Ok(())
                        }
                        OpCode::$c_get => {
                            let (idx_bits, _idx_kind) = self.pop_kinded()?;
                            let index = idx_bits as i64 as u32;
                            let (arr_bits, arr_kind) = self.pop_kinded()?;
                            let arr = arr_bits as usize as *const TypedArray<char>;
                            let len = unsafe { TypedArray::len(arr) };
                            let val = unsafe {
                                TypedArray::get(arr, index).ok_or(
                                    VMError::IndexOutOfBounds {
                                        index: index as i32,
                                        length: len as usize,
                                    },
                                )?
                            };
                            drop_with_kind(arr_bits, arr_kind);
                            // Codepoint pushed as inline bits per
                            // §2.7.6/Q8 KindedSlot::from_char shape.
                            self.push_kinded(val as u32 as u64, $c_kind)?;
                            Ok(())
                        }
                        OpCode::$c_push => {
                            let (val_bits, _vk) = self.pop_kinded()?;
                            // Codepoint validity check —
                            // `char::from_u32` rejects surrogates and
                            // out-of-range values; a corrupt slot
                            // surfaces a structured error instead of
                            // panicking on `unwrap`.
                            let val = char::from_u32(val_bits as u32).ok_or_else(|| {
                                VMError::RuntimeError(format!(
                                    "{}: invalid char codepoint 0x{:X}",
                                    $c_push_label, val_bits as u32
                                ))
                            })?;
                            let (arr_bits, arr_kind) = self.pop_kinded()?;
                            let arr = arr_bits as usize as *mut TypedArray<char>;
                            unsafe { TypedArray::push(arr, val); }
                            drop_with_kind(arr_bits, arr_kind);
                            Ok(())
                        }
                        OpCode::$c_set => {
                            let (val_bits, _vk) = self.pop_kinded()?;
                            let val = char::from_u32(val_bits as u32).ok_or_else(|| {
                                VMError::RuntimeError(format!(
                                    "{}: invalid char codepoint 0x{:X}",
                                    $c_set_label, val_bits as u32
                                ))
                            })?;
                            let (idx_bits, _ik) = self.pop_kinded()?;
                            let index = idx_bits as i64 as u32;
                            let (arr_bits, arr_kind) = self.pop_kinded()?;
                            let arr = arr_bits as usize as *mut TypedArray<char>;
                            unsafe { TypedArray::set(arr, index, val); }
                            drop_with_kind(arr_bits, arr_kind);
                            Ok(())
                        }
                    )+

                    $(
                        OpCode::$h_new => {
                            let cap = match instruction.operand {
                                Some(Operand::Count(n)) => n as u32,
                                _ => 0,
                            };
                            let ptr =
                                TypedArray::<*const $h_obj>::with_capacity(cap);
                            unsafe { stamp_elem_type(ptr as *mut u8, $h_tag) };
                            self.push_kinded(
                                ptr as usize as u64,
                                NativeKind::Ptr(HeapKind::TypedArray),
                            )?;
                            Ok(())
                        }
                        OpCode::$h_get => {
                            let (idx_bits, _idx_kind) = self.pop_kinded()?;
                            let index = idx_bits as i64 as u32;
                            let (arr_bits, arr_kind) = self.pop_kinded()?;
                            let arr =
                                arr_bits as usize as *const TypedArray<*const $h_obj>;
                            let len = unsafe { TypedArray::len(arr) };
                            let elem_ptr = unsafe {
                                TypedArray::<*const $h_obj>::get(arr, index).ok_or(
                                    VMError::IndexOutOfBounds {
                                        index: index as i32,
                                        length: len as usize,
                                    },
                                )?
                            };
                            // Retain the per-element header: array
                            // keeps its share, caller gets a fresh
                            // share released via the $h_kind arm in
                            // drop_with_kind.
                            unsafe { v2_retain(&(*elem_ptr).header) };
                            drop_with_kind(arr_bits, arr_kind);
                            self.push_kinded(elem_ptr as u64, $h_kind)?;
                            Ok(())
                        }
                        OpCode::$h_push => {
                            let (val_bits, val_kind) = self.pop_kinded()?;
                            if val_kind != $h_kind {
                                return Err(VMError::RuntimeError(format!(
                                    "{}: expected {:?}, got {:?}",
                                    $h_err, $h_kind, val_kind
                                )));
                            }
                            let val = val_bits as usize as *const $h_obj;
                            let (arr_bits, arr_kind) = self.pop_kinded()?;
                            let arr =
                                arr_bits as usize as *mut TypedArray<*const $h_obj>;
                            // Caller transfers their refcount share to
                            // the array (no retain).
                            unsafe { TypedArray::push(arr, val); }
                            drop_with_kind(arr_bits, arr_kind);
                            Ok(())
                        }
                        OpCode::$h_set => {
                            let (val_bits, val_kind) = self.pop_kinded()?;
                            if val_kind != $h_kind {
                                return Err(VMError::RuntimeError(format!(
                                    "{}: expected {:?}, got {:?}",
                                    $h_err, $h_kind, val_kind
                                )));
                            }
                            let val = val_bits as usize as *const $h_obj;
                            let (idx_bits, _ik) = self.pop_kinded()?;
                            let index = idx_bits as i64 as u32;
                            let (arr_bits, arr_kind) = self.pop_kinded()?;
                            let arr =
                                arr_bits as usize as *mut TypedArray<*const $h_obj>;
                            unsafe {
                                let old_ptr =
                                    TypedArray::<*const $h_obj>::get_unchecked(arr, index);
                                <$h_obj as HeapElement>::release_elem(old_ptr);
                                TypedArray::set(arr, index, val);
                            }
                            drop_with_kind(arr_bits, arr_kind);
                            Ok(())
                        }
                    )+

                    // ── Wave 3 Stabilize Round 1 V3-A2-followup-producer-cascade (2026-05-15) ──
                    //
                    // v2-raw heap-element literal constructors. Read the
                    // source value from the program constant / string
                    // pool, allocate a fresh `StringObj` / `DecimalObj`
                    // (refcount = 1), push the raw pointer bits with
                    // `NativeKind::StringV2` / `NativeKind::DecimalV2`.
                    // The caller's share is then transferred to the
                    // typed array on the subsequent
                    // `TypedArrayPushString` / `TypedArrayPushDecimal`
                    // (matches the per-element refcount discipline of
                    // the heap-row Get/Push/Set arms above).
                    //
                    // Per ADR-006 §2.7.5 stamp-at-compile-time: the
                    // kind is proven at compile-time emission; no
                    // runtime decode/probe at the FFI boundary.

                    // ── StringElem J.5d hand-written String heap row (2026-06-16) ──
                    //
                    // Extracted from the generic `heap_rows:` macro so Push/Set
                    // can accept BOTH carriers:
                    //   - `NativeKind::StringV2`: v2-raw `*const StringObj` —
                    //     transfer the caller's share to the array as-is (the
                    //     literal NewStringV2 contract).
                    //   - `NativeKind::String`: Phase-2c `Arc<String>` from
                    //     non-literal producers (`s + "!"`, split/join, f-string).
                    //     Materialize a fresh refcount-1 `StringObj` (copies the
                    //     bytes), store it, then release the consumed `Arc<String>`
                    //     share exactly once via `drop_with_kind(.., String)`.
                    //
                    // String and StringV2 remain DISTINCT NativeKind
                    // discriminators (CLAUDE.md Parallel-impl) — only the output
                    // `TypedArray<*const StringObj>` carrier is shared, via a real
                    // allocation at the storage boundary. New/Get mirror the
                    // generic heap-row template verbatim. This gating is String-
                    // only; Decimal/TypedObject/TraitObject/Nested stay strict.
                    OpCode::NewTypedArrayString => {
                        let cap = match instruction.operand {
                            Some(Operand::Count(n)) => n as u32,
                            _ => 0,
                        };
                        let ptr = TypedArray::<*const StringObj>::with_capacity(cap);
                        unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_STRING) };
                        self.push_kinded(
                            ptr as usize as u64,
                            NativeKind::Ptr(HeapKind::TypedArray),
                        )?;
                        Ok(())
                    }
                    OpCode::TypedArrayGetString => {
                        let (idx_bits, _idx_kind) = self.pop_kinded()?;
                        let index = idx_bits as i64 as u32;
                        let (arr_bits, arr_kind) = self.pop_kinded()?;
                        let arr = arr_bits as usize as *const TypedArray<*const StringObj>;
                        let len = unsafe { TypedArray::len(arr) };
                        let elem_ptr = unsafe {
                            TypedArray::<*const StringObj>::get(arr, index).ok_or(
                                VMError::IndexOutOfBounds {
                                    index: index as i32,
                                    length: len as usize,
                                },
                            )?
                        };
                        // Retain the per-element header: array keeps its share,
                        // caller gets a fresh share released via the StringV2 arm
                        // in drop_with_kind.
                        unsafe { v2_retain(&(*elem_ptr).header) };
                        drop_with_kind(arr_bits, arr_kind);
                        self.push_kinded(elem_ptr as u64, NativeKind::StringV2)?;
                        Ok(())
                    }
                    OpCode::TypedArrayPushString => {
                        let (val_bits, val_kind) = self.pop_kinded()?;
                        let (arr_bits, arr_kind) = self.pop_kinded()?;
                        let arr = arr_bits as usize as *mut TypedArray<*const StringObj>;
                        match val_kind {
                            NativeKind::StringV2 => {
                                let val = val_bits as usize as *const StringObj;
                                // Caller transfers their share to the array.
                                unsafe { TypedArray::push(arr, val); }
                            }
                            NativeKind::String => {
                                // SAFETY: bits = Arc::into_raw(Arc<String>); borrow &str.
                                let s: &str = unsafe { &*(val_bits as usize as *const String) };
                                let val = StringObj::new(s); // fresh refcount-1, copies bytes
                                unsafe { TypedArray::push(arr, val); }
                                // Release the consumed Arc share exactly once.
                                drop_with_kind(val_bits, NativeKind::String);
                            }
                            _ => {
                                drop_with_kind(arr_bits, arr_kind);
                                return Err(VMError::RuntimeError(format!(
                                    "TypedArrayPushString: expected StringV2 or String, got {:?}",
                                    val_kind
                                )));
                            }
                        }
                        drop_with_kind(arr_bits, arr_kind);
                        Ok(())
                    }
                    OpCode::TypedArraySetString => {
                        let (val_bits, val_kind) = self.pop_kinded()?;
                        let (idx_bits, _ik) = self.pop_kinded()?;
                        let index = idx_bits as i64 as u32;
                        let (arr_bits, arr_kind) = self.pop_kinded()?;
                        let arr = arr_bits as usize as *mut TypedArray<*const StringObj>;
                        let new_ptr: *const StringObj = match val_kind {
                            NativeKind::StringV2 => val_bits as usize as *const StringObj,
                            NativeKind::String => {
                                // SAFETY: bits = Arc::into_raw(Arc<String>); borrow &str.
                                let s: &str = unsafe { &*(val_bits as usize as *const String) };
                                let p = StringObj::new(s); // fresh refcount-1, copies bytes
                                drop_with_kind(val_bits, NativeKind::String);
                                p
                            }
                            _ => {
                                drop_with_kind(arr_bits, arr_kind);
                                return Err(VMError::RuntimeError(format!(
                                    "TypedArraySetString: expected StringV2 or String, got {:?}",
                                    val_kind
                                )));
                            }
                        };
                        unsafe {
                            let old_ptr =
                                TypedArray::<*const StringObj>::get_unchecked(arr, index);
                            <StringObj as HeapElement>::release_elem(old_ptr);
                            TypedArray::set(arr, index, new_ptr);
                        }
                        drop_with_kind(arr_bits, arr_kind);
                        Ok(())
                    }

                    OpCode::NewStringV2 => {
                        let str_id = match instruction.operand {
                            Some(Operand::Property(id)) => id as usize,
                            Some(Operand::Const(id)) => id as usize,
                            _ => {
                                return Err(VMError::RuntimeError(
                                    "NewStringV2 requires a Property/Const string-id operand"
                                        .to_string(),
                                ));
                            }
                        };
                        let s = self
                            .program
                            .strings
                            .get(str_id)
                            .ok_or_else(|| {
                                VMError::RuntimeError(format!(
                                    "NewStringV2: string id {} out of bounds (pool len = {})",
                                    str_id,
                                    self.program.strings.len()
                                ))
                            })?
                            .clone();
                        let ptr = StringObj::new(&s);
                        self.push_kinded(ptr as usize as u64, NativeKind::StringV2)?;
                        Ok(())
                    }

                    OpCode::NewDecimalV2 => {
                        let const_id = match instruction.operand {
                            Some(Operand::Const(id)) => id as usize,
                            _ => {
                                return Err(VMError::RuntimeError(
                                    "NewDecimalV2 requires a Const constant-id operand"
                                        .to_string(),
                                ));
                            }
                        };
                        let constant = self.program.constants.get(const_id).ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "NewDecimalV2: constant id {} out of bounds (pool len = {})",
                                const_id,
                                self.program.constants.len()
                            ))
                        })?;
                        let d = match constant {
                            crate::bytecode::Constant::Decimal(d) => *d,
                            other => {
                                return Err(VMError::RuntimeError(format!(
                                    "NewDecimalV2: expected Constant::Decimal, got {:?}",
                                    other
                                )));
                            }
                        };
                        let ptr = DecimalObj::new(d);
                        self.push_kinded(ptr as usize as u64, NativeKind::DecimalV2)?;
                        Ok(())
                    }

                    // ── Length ────────────────────────────────────────

                    OpCode::TypedArrayLen => {
                        let (arr_bits, arr_kind) = self.pop_kinded()?;
                        let arr = arr_bits as usize as *const TypedArray<u8>;
                        // len field is at a fixed offset regardless of T —
                        // safe to read via any T.
                        let len = unsafe { TypedArray::len(arr) };
                        drop_with_kind(arr_bits, arr_kind);
                        self.push_kinded(len as u64, NativeKind::Int64)?;
                        Ok(())
                    }

                    _ => Err(VMError::NotImplemented(format!(
                        "v2 typed array opcode {:?} not implemented",
                        instruction.opcode
                    ))),
                }
            }
        }
    };
}

// ─────────────────────────────────────────────────────────────────────────
// Macro invocation — emits `impl VirtualMachine { fn exec_v2_typed_array
// … }` with all per-kind arms inlined.
// ─────────────────────────────────────────────────────────────────────────

define_exec_v2_typed_array! {
    scalar_rows: [
        {
            ops: NewTypedArrayF64 / TypedArrayGetF64
                / TypedArrayPushF64 / TypedArraySetF64,
            storage: f64,
            elem_tag: ELEM_TYPE_F64,
            result_kind: NativeKind::Float64,
            decode(vb) => f64::from_bits(vb),
            encode(v: f64) => v.to_bits(),
        }
        {
            ops: NewTypedArrayI64 / TypedArrayGetI64
                / TypedArrayPushI64 / TypedArraySetI64,
            storage: i64,
            elem_tag: ELEM_TYPE_I64,
            result_kind: NativeKind::Int64,
            decode(vb) => vb as i64,
            encode(v: i64) => v as u64,
        }
        {
            ops: NewTypedArrayI32 / TypedArrayGetI32
                / TypedArrayPushI32 / TypedArraySetI32,
            storage: i32,
            elem_tag: ELEM_TYPE_I32,
            result_kind: NativeKind::Int32,
            decode(vb) => vb as i64 as i32,
            encode(v: i32) => v as i64 as u64,
        }
        {
            ops: NewTypedArrayBool / TypedArrayGetBool
                / TypedArrayPushBool / TypedArraySetBool,
            storage: u8,
            elem_tag: ELEM_TYPE_BOOL,
            result_kind: NativeKind::Bool,
            decode(vb) => if vb != 0 { 1u8 } else { 0u8 },
            encode(v: u8) => (v != 0) as u64,
        }
        {
            ops: NewTypedArrayI8 / TypedArrayGetI8
                / TypedArrayPushI8 / TypedArraySetI8,
            storage: i8,
            elem_tag: ELEM_TYPE_I8,
            result_kind: NativeKind::Int8,
            decode(vb) => vb as i64 as i8,
            encode(v: i8) => v as i64 as u64,
        }
        {
            ops: NewTypedArrayU8 / TypedArrayGetU8
                / TypedArrayPushU8 / TypedArraySetU8,
            storage: u8,
            elem_tag: ELEM_TYPE_U8,
            result_kind: NativeKind::UInt8,
            decode(vb) => vb as u8,
            encode(v: u8) => v as u64,
        }
        {
            ops: NewTypedArrayI16 / TypedArrayGetI16
                / TypedArrayPushI16 / TypedArraySetI16,
            storage: i16,
            elem_tag: ELEM_TYPE_I16,
            result_kind: NativeKind::Int16,
            decode(vb) => vb as i64 as i16,
            encode(v: i16) => v as i64 as u64,
        }
        {
            ops: NewTypedArrayU16 / TypedArrayGetU16
                / TypedArrayPushU16 / TypedArraySetU16,
            storage: u16,
            elem_tag: ELEM_TYPE_U16,
            result_kind: NativeKind::UInt16,
            decode(vb) => vb as u16,
            encode(v: u16) => v as u64,
        }
        {
            ops: NewTypedArrayU32 / TypedArrayGetU32
                / TypedArrayPushU32 / TypedArraySetU32,
            storage: u32,
            elem_tag: ELEM_TYPE_U32,
            result_kind: NativeKind::UInt32,
            decode(vb) => vb as u32,
            encode(v: u32) => v as u64,
        }
        {
            ops: NewTypedArrayF32 / TypedArrayGetF32
                / TypedArrayPushF32 / TypedArraySetF32,
            storage: f32,
            elem_tag: ELEM_TYPE_F32,
            result_kind: NativeKind::Float32,
            decode(vb) => f32::from_bits(vb as u32),
            encode(v: f32) => v.to_bits() as u64,
        }
    ],
    char_rows: [
        {
            ops: NewTypedArrayChar / TypedArrayGetChar
                / TypedArrayPushChar / TypedArraySetChar,
            elem_tag: ELEM_TYPE_CHAR,
            result_kind: NativeKind::Char,
            push_label: "TypedArrayPushChar",
            set_label: "TypedArraySetChar",
        }
    ],
    heap_rows: [
        // NOTE: the String heap row is NOT macro-generated. Its Push/Set arms
        // must accept BOTH `NativeKind::StringV2` (v2-raw, transfer) AND
        // `NativeKind::String` (Phase-2c Arc<String>, materialize-a-fresh-
        // StringObj + release the consumed Arc) — StringElem J.5d 2026-06-16.
        // The generic macro arm only accepts the single strict `$h_kind`, so
        // the four String opcode arms (New/Get/Push/Set) are hand-written in
        // the macro body trailer below (see `OpCode::NewTypedArrayString` ..).
        // The remaining heap rows (Decimal/TypedObject/TraitObject/Nested)
        // stay strict-kind — do NOT loosen them.
        {
            ops: NewTypedArrayDecimal / TypedArrayGetDecimal
                / TypedArrayPushDecimal / TypedArraySetDecimal,
            heap_obj: DecimalObj,
            elem_kind: NativeKind::DecimalV2,
            elem_tag: ELEM_TYPE_DECIMAL,
            err_label: "TypedArrayPush/SetDecimal",
        }
        {
            ops: NewTypedArrayTypedObject / TypedArrayGetTypedObject
                / TypedArrayPushTypedObject / TypedArraySetTypedObject,
            heap_obj: TypedObjectStorage,
            elem_kind: NativeKind::Ptr(HeapKind::TypedObject),
            elem_tag: ELEM_TYPE_TYPED_OBJECT,
            err_label: "TypedArrayPush/SetTypedObject",
        }
        // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05) —
        // `Array<dyn Trait>` element carrier per ADR-006 §2.7.5 + §2.7.24
        // Q25.C. Mirror of the TypedObject heap row above; `heap_obj =
        // TraitObjectStorage` (the fat-pointer carrier `{ value: *const
        // TypedObjectStorage, vtable: Arc<VTable> }`). Its `HeapElement::
        // release_elem` (heap_value.rs:3092) calls `v2_release` on the
        // on-header refcount; on refcount=0 the inner `_drop` releases the
        // inner TypedObject share + the vtable Arc. Element values are
        // produced by `OpCode::BoxTraitObject` (already `_new`-allocated,
        // already labeled `Ptr(HeapKind::TraitObject)`), so the push strict-
        // kind check accepts them with no carrier translation.
        {
            ops: NewTypedArrayTraitObject / TypedArrayGetTraitObject
                / TypedArrayPushTraitObject / TypedArraySetTraitObject,
            heap_obj: TraitObjectStorage,
            elem_kind: NativeKind::Ptr(HeapKind::TraitObject),
            elem_tag: ELEM_TYPE_TRAIT_OBJECT,
            err_label: "TypedArrayPush/SetTraitObject",
        }
        // Construction strict-typing close (USER RULING 2026-06-05) —
        // nested-array element. `heap_obj = TypedArrayElem` is the
        // HeapHeader-view newtype over an inner `TypedArray<U>`; its
        // `HeapElement::release_elem` re-enters the kind-erased
        // `release_v2_typed_array` (reads the inner `_pad` discriminant).
        // Get-retain via `v2_retain(&(*elem_ptr).header)` works because the
        // inner array's HeapHeader is at offset 0. The element carrier kind
        // is `Ptr(HeapKind::TypedArray)` — the SAME kind the outer array uses
        // — so push/set's strict-kind check accepts inner-array pointers.
        {
            ops: NewTypedArrayNested / TypedArrayGetNested
                / TypedArrayPushNested / TypedArraySetNested,
            heap_obj: TypedArrayElem,
            elem_kind: NativeKind::Ptr(HeapKind::TypedArray),
            elem_tag: ELEM_TYPE_TYPED_ARRAY,
            err_label: "TypedArrayPush/SetNested",
        }
    ],
}
