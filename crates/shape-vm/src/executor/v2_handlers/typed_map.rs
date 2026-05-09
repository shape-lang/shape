//! VM executor handlers for v2 typed map opcodes (Phase 3.2).
//!
//! These handlers operate on `TypedMap<K, V>` raw pointers (NativeScalar
//! shape — non-Arc, custom heap allocation). Pointer bits flow through the
//! kinded API as `NativeKind::UInt64`. String keys flow as
//! `NativeKind::String` (bits = `Arc::into_raw(Arc<String>)`).
//!
//! For string-keyed maps the handlers allocate a temporary `StringObj` for
//! lookup keys (get/has/delete) and a fresh owned `StringObj` for inserted
//! keys (set). Inserted keys are retained inside the map; lookup keys are
//! dropped immediately after the call.
//!
//! ADR-006 §2.7.7 / Wave 6.5 cluster C.

use std::sync::Arc;

use crate::bytecode::{Instruction, OpCode};
use crate::executor::vm_impl::stack::drop_with_kind;
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_map::{
    TypedMap, TypedMapI64F64, TypedMapI64I64, TypedMapI64Ptr, TypedMapStringF64, TypedMapStringI64,
    TypedMapStringPtr,
};
use shape_value::{NativeKind, VMError};

use super::super::VirtualMachine;

/// Pop a string key from the stack and allocate a fresh `StringObj` for it.
/// Returns `(StringObj_ptr, owning_arc)` where the Arc holds the original
/// share — caller must `drop_with_kind(arc, kind)` after use.
///
/// The Arc reconstruction reads the string content; the StringObj is a
/// separate allocation owned by the caller.
#[inline]
fn pop_string_key(vm: &mut VirtualMachine) -> Result<Option<*mut StringObj>, VMError> {
    let (key_bits, key_kind) = vm.pop_kinded()?;
    let result = match key_kind {
        NativeKind::String | NativeKind::Ptr(shape_value::heap_value::HeapKind::String) => {
            let arc = unsafe { Arc::<String>::from_raw(key_bits as *const String) };
            let so = StringObj::new(arc.as_str());
            // Restore the share (we read by reference).
            let _ = Arc::into_raw(arc);
            Some(so)
        }
        _ => None,
    };
    drop_with_kind(key_bits, key_kind);
    Ok(result)
}

impl VirtualMachine {
    /// Execute a v2 typed map opcode.
    pub(crate) fn exec_v2_typed_map(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        match instruction.opcode {
            // ── Allocation ──────────────────────────────────────────
            // Map pointers are raw `*mut TypedMap*` values, NativeScalar
            // shape — pushed as `NativeKind::UInt64` (no refcount).
            OpCode::NewTypedMapStringF64 => {
                let ptr = TypedMapStringF64::new();
                self.push_kinded(ptr as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::NewTypedMapStringI64 => {
                let ptr = TypedMapStringI64::new();
                self.push_kinded(ptr as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::NewTypedMapStringPtr => {
                let ptr = TypedMapStringPtr::new();
                self.push_kinded(ptr as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::NewTypedMapI64F64 => {
                let ptr = TypedMapI64F64::new();
                self.push_kinded(ptr as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::NewTypedMapI64I64 => {
                let ptr = TypedMapI64I64::new();
                self.push_kinded(ptr as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::NewTypedMapI64Ptr => {
                let ptr = TypedMapI64Ptr::new();
                self.push_kinded(ptr as u64, NativeKind::UInt64)?;
                Ok(())
            }

            // ── String-keyed Get ────────────────────────────────────
            OpCode::TypedMapStringF64Get => {
                let temp_key = pop_string_key(self)?;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *const TypedMapStringF64;
                let result = if let Some(key) = temp_key {
                    let v = unsafe { TypedMap::get(map, key) };
                    unsafe { StringObj::drop(key) };
                    v
                } else {
                    None
                };
                drop_with_kind(map_bits, map_kind);
                match result {
                    Some(v) => self.push_kinded(v.to_bits(), NativeKind::Float64)?,
                    None => self.push_kinded(Self::NONE_BITS, NativeKind::Bool)?,
                }
                Ok(())
            }
            OpCode::TypedMapStringI64Get => {
                let temp_key = pop_string_key(self)?;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *const TypedMapStringI64;
                let result = if let Some(key) = temp_key {
                    let v = unsafe { TypedMap::get(map, key) };
                    unsafe { StringObj::drop(key) };
                    v
                } else {
                    None
                };
                drop_with_kind(map_bits, map_kind);
                match result {
                    Some(v) => self.push_kinded(v as u64, NativeKind::Int64)?,
                    None => self.push_kinded(Self::NONE_BITS, NativeKind::Bool)?,
                }
                Ok(())
            }
            OpCode::TypedMapStringPtrGet => {
                let temp_key = pop_string_key(self)?;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *const TypedMapStringPtr;
                let result = if let Some(key) = temp_key {
                    let v = unsafe { TypedMap::get(map, key) };
                    unsafe { StringObj::drop(key) };
                    v
                } else {
                    None
                };
                drop_with_kind(map_bits, map_kind);
                match result {
                    Some(p) => self.push_kinded(p as u64, NativeKind::UInt64)?,
                    None => self.push_kinded(Self::NONE_BITS, NativeKind::Bool)?,
                }
                Ok(())
            }

            // ── i64-keyed Get ───────────────────────────────────────
            OpCode::TypedMapI64F64Get => {
                let (key_bits, _key_kind) = self.pop_kinded()?;
                let key = key_bits as i64;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *const TypedMapI64F64;
                let result = unsafe { TypedMapI64F64::get_i64(map, key) };
                drop_with_kind(map_bits, map_kind);
                match result {
                    Some(v) => self.push_kinded(v.to_bits(), NativeKind::Float64)?,
                    None => self.push_kinded(Self::NONE_BITS, NativeKind::Bool)?,
                }
                Ok(())
            }
            OpCode::TypedMapI64I64Get => {
                let (key_bits, _key_kind) = self.pop_kinded()?;
                let key = key_bits as i64;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *const TypedMapI64I64;
                let result = unsafe { TypedMapI64I64::get_i64(map, key) };
                drop_with_kind(map_bits, map_kind);
                match result {
                    Some(v) => self.push_kinded(v as u64, NativeKind::Int64)?,
                    None => self.push_kinded(Self::NONE_BITS, NativeKind::Bool)?,
                }
                Ok(())
            }
            OpCode::TypedMapI64PtrGet => {
                let (key_bits, _key_kind) = self.pop_kinded()?;
                let key = key_bits as i64;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *const TypedMapI64Ptr;
                let result = unsafe { TypedMapI64Ptr::get_i64(map, key) };
                drop_with_kind(map_bits, map_kind);
                match result {
                    Some(p) => self.push_kinded(p as u64, NativeKind::UInt64)?,
                    None => self.push_kinded(Self::NONE_BITS, NativeKind::Bool)?,
                }
                Ok(())
            }

            // ── String-keyed Set ────────────────────────────────────
            OpCode::TypedMapStringF64Set => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = f64::from_bits(val_bits);
                let temp_key = pop_string_key(self)?;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *mut TypedMapStringF64;
                if let Some(key) = temp_key {
                    unsafe {
                        let _ = TypedMap::insert(map, key, val);
                    }
                }
                drop_with_kind(map_bits, map_kind);
                Ok(())
            }
            OpCode::TypedMapStringI64Set => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as i64;
                let temp_key = pop_string_key(self)?;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *mut TypedMapStringI64;
                if let Some(key) = temp_key {
                    unsafe {
                        let _ = TypedMap::insert(map, key, val);
                    }
                }
                drop_with_kind(map_bits, map_kind);
                Ok(())
            }
            OpCode::TypedMapStringPtrSet => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as *const u8;
                let temp_key = pop_string_key(self)?;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *mut TypedMapStringPtr;
                if let Some(key) = temp_key {
                    unsafe {
                        let _ = TypedMap::insert(map, key, val);
                    }
                }
                drop_with_kind(map_bits, map_kind);
                Ok(())
            }

            // ── i64-keyed Set ───────────────────────────────────────
            OpCode::TypedMapI64F64Set => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = f64::from_bits(val_bits);
                let (key_bits, _kk) = self.pop_kinded()?;
                let key = key_bits as i64;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *mut TypedMapI64F64;
                unsafe {
                    let _ = TypedMapI64F64::insert_i64(map, key, val);
                }
                drop_with_kind(map_bits, map_kind);
                Ok(())
            }
            OpCode::TypedMapI64I64Set => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as i64;
                let (key_bits, _kk) = self.pop_kinded()?;
                let key = key_bits as i64;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *mut TypedMapI64I64;
                unsafe {
                    let _ = TypedMapI64I64::insert_i64(map, key, val);
                }
                drop_with_kind(map_bits, map_kind);
                Ok(())
            }
            OpCode::TypedMapI64PtrSet => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as *const u8;
                let (key_bits, _kk) = self.pop_kinded()?;
                let key = key_bits as i64;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *mut TypedMapI64Ptr;
                unsafe {
                    let _ = TypedMapI64Ptr::insert_i64(map, key, val);
                }
                drop_with_kind(map_bits, map_kind);
                Ok(())
            }

            // ── String-keyed Has ────────────────────────────────────
            OpCode::TypedMapStringF64Has => {
                let temp_key = pop_string_key(self)?;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *const TypedMapStringF64;
                let present = if let Some(key) = temp_key {
                    let p = unsafe { TypedMap::contains_key(map, key) };
                    unsafe { StringObj::drop(key) };
                    p
                } else {
                    false
                };
                drop_with_kind(map_bits, map_kind);
                self.push_kinded(present as u64, NativeKind::Bool)?;
                Ok(())
            }
            OpCode::TypedMapStringI64Has => {
                let temp_key = pop_string_key(self)?;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *const TypedMapStringI64;
                let present = if let Some(key) = temp_key {
                    let p = unsafe { TypedMap::contains_key(map, key) };
                    unsafe { StringObj::drop(key) };
                    p
                } else {
                    false
                };
                drop_with_kind(map_bits, map_kind);
                self.push_kinded(present as u64, NativeKind::Bool)?;
                Ok(())
            }
            OpCode::TypedMapStringPtrHas => {
                let temp_key = pop_string_key(self)?;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *const TypedMapStringPtr;
                let present = if let Some(key) = temp_key {
                    let p = unsafe { TypedMap::contains_key(map, key) };
                    unsafe { StringObj::drop(key) };
                    p
                } else {
                    false
                };
                drop_with_kind(map_bits, map_kind);
                self.push_kinded(present as u64, NativeKind::Bool)?;
                Ok(())
            }

            // ── i64-keyed Has ───────────────────────────────────────
            OpCode::TypedMapI64F64Has => {
                let (key_bits, _kk) = self.pop_kinded()?;
                let key = key_bits as i64;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *const TypedMapI64F64;
                let present = unsafe { TypedMapI64F64::contains_key_i64(map, key) };
                drop_with_kind(map_bits, map_kind);
                self.push_kinded(present as u64, NativeKind::Bool)?;
                Ok(())
            }
            OpCode::TypedMapI64I64Has => {
                let (key_bits, _kk) = self.pop_kinded()?;
                let key = key_bits as i64;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *const TypedMapI64I64;
                let present = unsafe { TypedMapI64I64::contains_key_i64(map, key) };
                drop_with_kind(map_bits, map_kind);
                self.push_kinded(present as u64, NativeKind::Bool)?;
                Ok(())
            }
            OpCode::TypedMapI64PtrHas => {
                let (key_bits, _kk) = self.pop_kinded()?;
                let key = key_bits as i64;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *const TypedMapI64Ptr;
                let present = unsafe { TypedMapI64Ptr::contains_key_i64(map, key) };
                drop_with_kind(map_bits, map_kind);
                self.push_kinded(present as u64, NativeKind::Bool)?;
                Ok(())
            }

            // ── String-keyed Delete ────────────────────────────────
            OpCode::TypedMapStringF64Delete => {
                let temp_key = pop_string_key(self)?;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *mut TypedMapStringF64;
                if let Some(key) = temp_key {
                    unsafe {
                        let _ = TypedMap::remove(map, key);
                        StringObj::drop(key);
                    }
                }
                drop_with_kind(map_bits, map_kind);
                Ok(())
            }
            OpCode::TypedMapStringI64Delete => {
                let temp_key = pop_string_key(self)?;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *mut TypedMapStringI64;
                if let Some(key) = temp_key {
                    unsafe {
                        let _ = TypedMap::remove(map, key);
                        StringObj::drop(key);
                    }
                }
                drop_with_kind(map_bits, map_kind);
                Ok(())
            }
            OpCode::TypedMapStringPtrDelete => {
                let temp_key = pop_string_key(self)?;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *mut TypedMapStringPtr;
                if let Some(key) = temp_key {
                    unsafe {
                        let _ = TypedMap::remove(map, key);
                        StringObj::drop(key);
                    }
                }
                drop_with_kind(map_bits, map_kind);
                Ok(())
            }

            // ── i64-keyed Delete ───────────────────────────────────
            OpCode::TypedMapI64F64Delete => {
                let (key_bits, _kk) = self.pop_kinded()?;
                let key = key_bits as i64;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *mut TypedMapI64F64;
                unsafe {
                    let _ = TypedMapI64F64::remove_i64(map, key);
                }
                drop_with_kind(map_bits, map_kind);
                Ok(())
            }
            OpCode::TypedMapI64I64Delete => {
                let (key_bits, _kk) = self.pop_kinded()?;
                let key = key_bits as i64;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *mut TypedMapI64I64;
                unsafe {
                    let _ = TypedMapI64I64::remove_i64(map, key);
                }
                drop_with_kind(map_bits, map_kind);
                Ok(())
            }
            OpCode::TypedMapI64PtrDelete => {
                let (key_bits, _kk) = self.pop_kinded()?;
                let key = key_bits as i64;
                let (map_bits, map_kind) = self.pop_kinded()?;
                let map = map_bits as *mut TypedMapI64Ptr;
                unsafe {
                    let _ = TypedMapI64Ptr::remove_i64(map, key);
                }
                drop_with_kind(map_bits, map_kind);
                Ok(())
            }

            _ => Err(VMError::NotImplemented(format!(
                "v2 typed map opcode {:?} not implemented",
                instruction.opcode
            ))),
        }
    }
}
