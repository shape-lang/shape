//! VM executor handlers for typed map opcodes.
//!
//! These handlers operate on `TypedMap<K, V>` pointers stored as raw `u64`
//! values directly on the stack — no `ValueWord` boxing. The key/value types
//! are encoded into the opcode itself, so the handlers do zero runtime tag
//! dispatch.
//!
//! For string-keyed maps the handlers allocate a temporary `StringObj` for
//! lookup keys (get/has/delete) and a fresh owned `StringObj` for inserted
//! keys (set). Inserted keys are retained inside the map; lookup keys are
//! dropped immediately after the call.

#![allow(dead_code)]

use shape_value::native::string_obj::StringObj;
use shape_value::native::typed_map::{
    TypedMap, TypedMapI64F64, TypedMapI64I64, TypedMapI64Ptr, TypedMapStringF64, TypedMapStringI64,
    TypedMapStringPtr,
};
use shape_value::value_word::ValueWordExt;
use shape_value::{VMError, ValueWord, ValueWordExt};

use super::super::VirtualMachine;

/// Allocate a temporary `StringObj` from a ValueWord-held string. The caller
/// is responsible for `StringObj::drop`-ing the result when done. Returns
/// `None` for ValueWords that are not strings.
#[inline]
fn alloc_temp_string_key(vw: &ValueWord) -> Option<*mut StringObj> {
    let s = vw.as_str()?;
    Some(StringObj::new(s))
}

// ---------------------------------------------------------------------------
// OpCode-gated dispatch — typed map opcodes are now defined.
// ---------------------------------------------------------------------------

use crate::bytecode::{Instruction, OpCode};

impl VirtualMachine {
    /// Execute a typed map opcode.
    pub(crate) fn exec_typed_map(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        match instruction.opcode {
                // ── Allocation ──────────────────────────────────────────
                OpCode::NewTypedMapStringF64 => {
                    let ptr = TypedMapStringF64::new();
                    self.push_raw_u64(ptr as u64)?;
                    Ok(())
                }
                OpCode::NewTypedMapStringI64 => {
                    let ptr = TypedMapStringI64::new();
                    self.push_raw_u64(ptr as u64)?;
                    Ok(())
                }
                OpCode::NewTypedMapStringPtr => {
                    let ptr = TypedMapStringPtr::new();
                    self.push_raw_u64(ptr as u64)?;
                    Ok(())
                }
                OpCode::NewTypedMapI64F64 => {
                    let ptr = TypedMapI64F64::new();
                    self.push_raw_u64(ptr as u64)?;
                    Ok(())
                }
                OpCode::NewTypedMapI64I64 => {
                    let ptr = TypedMapI64I64::new();
                    self.push_raw_u64(ptr as u64)?;
                    Ok(())
                }
                OpCode::NewTypedMapI64Ptr => {
                    let ptr = TypedMapI64Ptr::new();
                    self.push_raw_u64(ptr as u64)?;
                    Ok(())
                }

                // ── String-keyed Get ────────────────────────────────────
                OpCode::TypedMapStringF64Get => {
                    let key_vw = self.pop_raw_u64()?;
                    let map = self.pop_raw_u64()? as *const TypedMapStringF64;
                    let result = if let Some(key) = alloc_temp_string_key(&key_vw) {
                        let v = unsafe { TypedMap::get(map, key) };
                        unsafe { StringObj::drop(key) };
                        v
                    } else {
                        None
                    };
                    match result {
                        Some(v) => self.push_raw_f64(v)?,
                        None => self.push_raw_u64(Self::NONE_BITS)?,
                    }
                    Ok(())
                }
                OpCode::TypedMapStringI64Get => {
                    let key_vw = self.pop_raw_u64()?;
                    let map = self.pop_raw_u64()? as *const TypedMapStringI64;
                    let result = if let Some(key) = alloc_temp_string_key(&key_vw) {
                        let v = unsafe { TypedMap::get(map, key) };
                        unsafe { StringObj::drop(key) };
                        v
                    } else {
                        None
                    };
                    match result {
                        Some(v) => self.push_raw_i64(v)?,
                        None => self.push_raw_u64(Self::NONE_BITS)?,
                    }
                    Ok(())
                }
                OpCode::TypedMapStringPtrGet => {
                    let key_vw = self.pop_raw_u64()?;
                    let map = self.pop_raw_u64()? as *const TypedMapStringPtr;
                    let result = if let Some(key) = alloc_temp_string_key(&key_vw) {
                        let v = unsafe { TypedMap::get(map, key) };
                        unsafe { StringObj::drop(key) };
                        v
                    } else {
                        None
                    };
                    match result {
                        Some(p) => self.push_raw_u64(p as u64)?,
                        None => self.push_raw_u64(Self::NONE_BITS)?,
                    }
                    Ok(())
                }

                // ── i64-keyed Get ───────────────────────────────────────
                OpCode::TypedMapI64F64Get => {
                    let key = self.pop_raw_i64()?;
                    let map = self.pop_raw_u64()? as *const TypedMapI64F64;
                    match unsafe { TypedMapI64F64::get_i64(map, key) } {
                        Some(v) => self.push_raw_f64(v)?,
                        None => self.push_raw_u64(Self::NONE_BITS)?,
                    }
                    Ok(())
                }
                OpCode::TypedMapI64I64Get => {
                    let key = self.pop_raw_i64()?;
                    let map = self.pop_raw_u64()? as *const TypedMapI64I64;
                    match unsafe { TypedMapI64I64::get_i64(map, key) } {
                        Some(v) => self.push_raw_i64(v)?,
                        None => self.push_raw_u64(Self::NONE_BITS)?,
                    }
                    Ok(())
                }
                OpCode::TypedMapI64PtrGet => {
                    let key = self.pop_raw_i64()?;
                    let map = self.pop_raw_u64()? as *const TypedMapI64Ptr;
                    match unsafe { TypedMapI64Ptr::get_i64(map, key) } {
                        Some(p) => self.push_raw_u64(p as u64)?,
                        None => self.push_raw_u64(Self::NONE_BITS)?,
                    }
                    Ok(())
                }

                // ── String-keyed Set ────────────────────────────────────
                OpCode::TypedMapStringF64Set => {
                    let val = self.pop_raw_f64()?;
                    let key_vw = self.pop_raw_u64()?;
                    let map = self.pop_raw_u64()? as *mut TypedMapStringF64;
                    if let Some(key_str) = key_vw.as_str() {
                        let key = StringObj::new(key_str);
                        unsafe {
                            let _ = TypedMap::insert(map, key, val);
                        }
                    }
                    Ok(())
                }
                OpCode::TypedMapStringI64Set => {
                    let val = self.pop_raw_i64()?;
                    let key_vw = self.pop_raw_u64()?;
                    let map = self.pop_raw_u64()? as *mut TypedMapStringI64;
                    if let Some(key_str) = key_vw.as_str() {
                        let key = StringObj::new(key_str);
                        unsafe {
                            let _ = TypedMap::insert(map, key, val);
                        }
                    }
                    Ok(())
                }
                OpCode::TypedMapStringPtrSet => {
                    let val = self.pop_raw_u64()? as *const u8;
                    let key_vw = self.pop_raw_u64()?;
                    let map = self.pop_raw_u64()? as *mut TypedMapStringPtr;
                    if let Some(key_str) = key_vw.as_str() {
                        let key = StringObj::new(key_str);
                        unsafe {
                            let _ = TypedMap::insert(map, key, val);
                        }
                    }
                    Ok(())
                }

                // ── i64-keyed Set ───────────────────────────────────────
                OpCode::TypedMapI64F64Set => {
                    let val = self.pop_raw_f64()?;
                    let key = self.pop_raw_i64()?;
                    let map = self.pop_raw_u64()? as *mut TypedMapI64F64;
                    unsafe {
                        let _ = TypedMapI64F64::insert_i64(map, key, val);
                    }
                    Ok(())
                }
                OpCode::TypedMapI64I64Set => {
                    let val = self.pop_raw_i64()?;
                    let key = self.pop_raw_i64()?;
                    let map = self.pop_raw_u64()? as *mut TypedMapI64I64;
                    unsafe {
                        let _ = TypedMapI64I64::insert_i64(map, key, val);
                    }
                    Ok(())
                }
                OpCode::TypedMapI64PtrSet => {
                    let val = self.pop_raw_u64()? as *const u8;
                    let key = self.pop_raw_i64()?;
                    let map = self.pop_raw_u64()? as *mut TypedMapI64Ptr;
                    unsafe {
                        let _ = TypedMapI64Ptr::insert_i64(map, key, val);
                    }
                    Ok(())
                }

                // ── String-keyed Has ────────────────────────────────────
                OpCode::TypedMapStringF64Has => {
                    let key_vw = self.pop_raw_u64()?;
                    let map = self.pop_raw_u64()? as *const TypedMapStringF64;
                    let present = if let Some(key) = alloc_temp_string_key(&key_vw) {
                        let p = unsafe { TypedMap::contains_key(map, key) };
                        unsafe { StringObj::drop(key) };
                        p
                    } else {
                        false
                    };
                    self.push_raw_bool(present)?;
                    Ok(())
                }
                OpCode::TypedMapStringI64Has => {
                    let key_vw = self.pop_raw_u64()?;
                    let map = self.pop_raw_u64()? as *const TypedMapStringI64;
                    let present = if let Some(key) = alloc_temp_string_key(&key_vw) {
                        let p = unsafe { TypedMap::contains_key(map, key) };
                        unsafe { StringObj::drop(key) };
                        p
                    } else {
                        false
                    };
                    self.push_raw_bool(present)?;
                    Ok(())
                }
                OpCode::TypedMapStringPtrHas => {
                    let key_vw = self.pop_raw_u64()?;
                    let map = self.pop_raw_u64()? as *const TypedMapStringPtr;
                    let present = if let Some(key) = alloc_temp_string_key(&key_vw) {
                        let p = unsafe { TypedMap::contains_key(map, key) };
                        unsafe { StringObj::drop(key) };
                        p
                    } else {
                        false
                    };
                    self.push_raw_bool(present)?;
                    Ok(())
                }

                // ── i64-keyed Has ───────────────────────────────────────
                OpCode::TypedMapI64F64Has => {
                    let key = self.pop_raw_i64()?;
                    let map = self.pop_raw_u64()? as *const TypedMapI64F64;
                    let present = unsafe { TypedMapI64F64::contains_key_i64(map, key) };
                    self.push_raw_bool(present)?;
                    Ok(())
                }
                OpCode::TypedMapI64I64Has => {
                    let key = self.pop_raw_i64()?;
                    let map = self.pop_raw_u64()? as *const TypedMapI64I64;
                    let present = unsafe { TypedMapI64I64::contains_key_i64(map, key) };
                    self.push_raw_bool(present)?;
                    Ok(())
                }
                OpCode::TypedMapI64PtrHas => {
                    let key = self.pop_raw_i64()?;
                    let map = self.pop_raw_u64()? as *const TypedMapI64Ptr;
                    let present = unsafe { TypedMapI64Ptr::contains_key_i64(map, key) };
                    self.push_raw_bool(present)?;
                    Ok(())
                }

                // ── String-keyed Delete ────────────────────────────────
                OpCode::TypedMapStringF64Delete => {
                    let key_vw = self.pop_raw_u64()?;
                    let map = self.pop_raw_u64()? as *mut TypedMapStringF64;
                    if let Some(key) = alloc_temp_string_key(&key_vw) {
                        unsafe {
                            let _ = TypedMap::remove(map, key);
                            StringObj::drop(key);
                        }
                    }
                    Ok(())
                }
                OpCode::TypedMapStringI64Delete => {
                    let key_vw = self.pop_raw_u64()?;
                    let map = self.pop_raw_u64()? as *mut TypedMapStringI64;
                    if let Some(key) = alloc_temp_string_key(&key_vw) {
                        unsafe {
                            let _ = TypedMap::remove(map, key);
                            StringObj::drop(key);
                        }
                    }
                    Ok(())
                }
                OpCode::TypedMapStringPtrDelete => {
                    let key_vw = self.pop_raw_u64()?;
                    let map = self.pop_raw_u64()? as *mut TypedMapStringPtr;
                    if let Some(key) = alloc_temp_string_key(&key_vw) {
                        unsafe {
                            let _ = TypedMap::remove(map, key);
                            StringObj::drop(key);
                        }
                    }
                    Ok(())
                }

                // ── i64-keyed Delete ───────────────────────────────────
                OpCode::TypedMapI64F64Delete => {
                    let key = self.pop_raw_i64()?;
                    let map = self.pop_raw_u64()? as *mut TypedMapI64F64;
                    unsafe {
                        let _ = TypedMapI64F64::remove_i64(map, key);
                    }
                    Ok(())
                }
                OpCode::TypedMapI64I64Delete => {
                    let key = self.pop_raw_i64()?;
                    let map = self.pop_raw_u64()? as *mut TypedMapI64I64;
                    unsafe {
                        let _ = TypedMapI64I64::remove_i64(map, key);
                    }
                    Ok(())
                }
                OpCode::TypedMapI64PtrDelete => {
                    let key = self.pop_raw_i64()?;
                    let map = self.pop_raw_u64()? as *mut TypedMapI64Ptr;
                    unsafe {
                        let _ = TypedMapI64Ptr::remove_i64(map, key);
                    }
                    Ok(())
                }

                _ => Err(VMError::NotImplemented(format!(
                    "typed map opcode {:?} not implemented",
                    instruction.opcode
                ))),
            }
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end smoke test: allocate a TypedMap<string, f64>, insert via the
    /// runtime helper, look up via the runtime helper, and confirm the value
    /// round-trips. This exercises the same code paths the VM handlers use.
    #[test]
    fn test_typed_map_string_f64_round_trip() {
        unsafe {
            let map = TypedMapStringF64::new();
            let k = StringObj::new("alpha");
            let _ = TypedMap::insert(map, k, 3.14_f64);

            let lookup = StringObj::new("alpha");
            let v = TypedMap::get(map, lookup);
            assert_eq!(v, Some(3.14_f64));

            StringObj::drop(lookup);
            // k is retained inside the map; drop_map will not free it because
            // the map only frees the bucket array. We leak the StringObj here
            // for test simplicity — production handlers face the same caveat
            // (the map borrows the key pointer, refcount-managed elsewhere).
            TypedMap::drop_map(map);
            // Drop k after drop_map to keep the bucket pointer valid until
            // drop_map releases the bucket array.
            StringObj::drop(k);
        }
    }

    #[test]
    fn test_typed_map_i64_f64_round_trip() {
        unsafe {
            let map = TypedMapI64F64::new();
            let _ = TypedMapI64F64::insert_i64(map, 42_i64, 2.718_f64);
            let v = TypedMapI64F64::get_i64(map, 42_i64);
            assert_eq!(v, Some(2.718_f64));
            assert!(!TypedMapI64F64::contains_key_i64(map, 99));
            let _ = TypedMapI64F64::remove_i64(map, 42_i64);
            assert_eq!(TypedMapI64F64::get_i64(map, 42_i64), None);
            TypedMap::drop_map(map);
        }
    }
}
