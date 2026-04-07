//! FFI-friendly C-ABI wrappers around `TypedMap<K, V>` for the JIT and VM.
//!
//! Each function uses `extern "C"` and operates on opaque `*mut u8` map
//! handles. String keys are passed as `(key_ptr: *const u8, key_len: u32)`
//! pointing into UTF-8 bytes — the wrapper allocates a transient `StringObj`
//! for hashing/comparison only; it does NOT take ownership of the key bytes
//! for non-mutating ops. For `set`, the wrapper allocates a refcounted
//! `StringObj` so the map can keep a stable key pointer.
//!
//! `get_*` returns `bool` (`true` if found) and writes the value via the
//! `out` pointer when present. `set_*` always succeeds (or aborts on OOM).
//! `delete_*` returns `true` if a value was removed.
//!
//! These wrappers are intentionally minimal — Agent 2 of Phase 3.2 builds
//! the VM/JIT integration on top.
//!
//! ## Naming convention
//!
//! ```text
//! typed_map_<keyty>_<valty>_<op>
//! ```
//!
//! `<op>` is one of: `alloc`, `get`, `set`, `delete`, `has`, `len`, `clear`, `drop`.

use super::string_obj::StringObj;
use super::typed_map::{
    TypedMap, TypedMapI64F64, TypedMapI64I64, TypedMapI64Ptr, TypedMapStringBool,
    TypedMapStringF64, TypedMapStringI64, TypedMapStringPtr,
};

// ---------------------------------------------------------------------------
// String-key helpers
// ---------------------------------------------------------------------------

/// Build a transient `StringObj` from raw `(ptr, len)` bytes for hashing/lookup.
///
/// # Safety
/// `key` must point to `key_len` valid bytes.
#[inline]
unsafe fn make_transient_key(key: *const u8, key_len: u32) -> *mut StringObj {
    let s = unsafe {
        if key_len == 0 {
            ""
        } else {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(key, key_len as usize))
        }
    };
    StringObj::new(s)
}

// ---------------------------------------------------------------------------
// (string, f64)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_f64_alloc() -> *mut u8 {
    TypedMapStringF64::new() as *mut u8
}

/// Returns `true` if the key was found; on success writes the value to `*out`.
///
/// # Safety
/// `map` must be a `TypedMapStringF64*`. `key` must point to `key_len` valid
/// bytes. `out` must be a writable `*mut f64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_f64_get(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
    out: *mut f64,
) -> bool {
    unsafe {
        let m = map as *const TypedMapStringF64;
        let tk = make_transient_key(key, key_len);
        let result = TypedMap::get(m, tk);
        StringObj::drop(tk);
        match result {
            Some(v) => {
                *out = v;
                true
            }
            None => false,
        }
    }
}

/// Insert/update a `(string, f64)` entry. The map takes ownership of the
/// allocated key string for new insertions.
///
/// # Safety
/// `map` must be a `TypedMapStringF64*`. `key` must point to `key_len` valid bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_f64_set(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
    value: f64,
) {
    unsafe {
        let m = map as *mut TypedMapStringF64;
        let owned_key = make_transient_key(key, key_len);
        if TypedMap::insert(m, owned_key, value).is_some() {
            // Key already existed; the existing key pointer was retained,
            // so the new one we just allocated is unused. Free it.
            StringObj::drop(owned_key);
        }
    }
}

/// Remove an entry. Returns `true` if a value was removed.
///
/// # Safety
/// `map` must be a `TypedMapStringF64*`. `key` must point to `key_len` valid bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_f64_delete(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
) -> bool {
    unsafe {
        let m = map as *mut TypedMapStringF64;
        let tk = make_transient_key(key, key_len);
        let result = TypedMap::remove(m, tk);
        StringObj::drop(tk);
        result.is_some()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_f64_has(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
) -> bool {
    unsafe {
        let m = map as *const TypedMapStringF64;
        let tk = make_transient_key(key, key_len);
        let result = TypedMap::contains_key(m, tk);
        StringObj::drop(tk);
        result
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_f64_len(map: *mut u8) -> u32 {
    unsafe { TypedMap::len(map as *const TypedMapStringF64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_f64_clear(map: *mut u8) {
    unsafe { TypedMap::clear(map as *mut TypedMapStringF64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_f64_drop(map: *mut u8) {
    unsafe { TypedMap::drop_map(map as *mut TypedMapStringF64) }
}

// ---------------------------------------------------------------------------
// (string, i64)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_i64_alloc() -> *mut u8 {
    TypedMapStringI64::new() as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_i64_get(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
    out: *mut i64,
) -> bool {
    unsafe {
        let m = map as *const TypedMapStringI64;
        let tk = make_transient_key(key, key_len);
        let result = TypedMap::get(m, tk);
        StringObj::drop(tk);
        match result {
            Some(v) => {
                *out = v;
                true
            }
            None => false,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_i64_set(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
    value: i64,
) {
    unsafe {
        let m = map as *mut TypedMapStringI64;
        let owned_key = make_transient_key(key, key_len);
        if TypedMap::insert(m, owned_key, value).is_some() {
            StringObj::drop(owned_key);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_i64_delete(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
) -> bool {
    unsafe {
        let m = map as *mut TypedMapStringI64;
        let tk = make_transient_key(key, key_len);
        let result = TypedMap::remove(m, tk);
        StringObj::drop(tk);
        result.is_some()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_i64_has(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
) -> bool {
    unsafe {
        let m = map as *const TypedMapStringI64;
        let tk = make_transient_key(key, key_len);
        let result = TypedMap::contains_key(m, tk);
        StringObj::drop(tk);
        result
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_i64_len(map: *mut u8) -> u32 {
    unsafe { TypedMap::len(map as *const TypedMapStringI64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_i64_clear(map: *mut u8) {
    unsafe { TypedMap::clear(map as *mut TypedMapStringI64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_i64_drop(map: *mut u8) {
    unsafe { TypedMap::drop_map(map as *mut TypedMapStringI64) }
}

// ---------------------------------------------------------------------------
// (string, bool)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_bool_alloc() -> *mut u8 {
    TypedMapStringBool::new() as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_bool_get(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
    out: *mut bool,
) -> bool {
    unsafe {
        let m = map as *const TypedMapStringBool;
        let tk = make_transient_key(key, key_len);
        let result = TypedMap::get(m, tk);
        StringObj::drop(tk);
        match result {
            Some(v) => {
                *out = v;
                true
            }
            None => false,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_bool_set(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
    value: bool,
) {
    unsafe {
        let m = map as *mut TypedMapStringBool;
        let owned_key = make_transient_key(key, key_len);
        if TypedMap::insert(m, owned_key, value).is_some() {
            StringObj::drop(owned_key);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_bool_delete(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
) -> bool {
    unsafe {
        let m = map as *mut TypedMapStringBool;
        let tk = make_transient_key(key, key_len);
        let result = TypedMap::remove(m, tk);
        StringObj::drop(tk);
        result.is_some()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_bool_has(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
) -> bool {
    unsafe {
        let m = map as *const TypedMapStringBool;
        let tk = make_transient_key(key, key_len);
        let result = TypedMap::contains_key(m, tk);
        StringObj::drop(tk);
        result
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_bool_len(map: *mut u8) -> u32 {
    unsafe { TypedMap::len(map as *const TypedMapStringBool) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_bool_clear(map: *mut u8) {
    unsafe { TypedMap::clear(map as *mut TypedMapStringBool) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_bool_drop(map: *mut u8) {
    unsafe { TypedMap::drop_map(map as *mut TypedMapStringBool) }
}

// ---------------------------------------------------------------------------
// (string, ptr)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_ptr_alloc() -> *mut u8 {
    TypedMapStringPtr::new() as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_ptr_get(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
    out: *mut *const u8,
) -> bool {
    unsafe {
        let m = map as *const TypedMapStringPtr;
        let tk = make_transient_key(key, key_len);
        let result = TypedMap::get(m, tk);
        StringObj::drop(tk);
        match result {
            Some(v) => {
                *out = v;
                true
            }
            None => false,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_ptr_set(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
    value: *const u8,
) {
    unsafe {
        let m = map as *mut TypedMapStringPtr;
        let owned_key = make_transient_key(key, key_len);
        if TypedMap::insert(m, owned_key, value).is_some() {
            StringObj::drop(owned_key);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_ptr_delete(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
) -> bool {
    unsafe {
        let m = map as *mut TypedMapStringPtr;
        let tk = make_transient_key(key, key_len);
        let result = TypedMap::remove(m, tk);
        StringObj::drop(tk);
        result.is_some()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_ptr_has(
    map: *mut u8,
    key: *const u8,
    key_len: u32,
) -> bool {
    unsafe {
        let m = map as *const TypedMapStringPtr;
        let tk = make_transient_key(key, key_len);
        let result = TypedMap::contains_key(m, tk);
        StringObj::drop(tk);
        result
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_ptr_len(map: *mut u8) -> u32 {
    unsafe { TypedMap::len(map as *const TypedMapStringPtr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_ptr_clear(map: *mut u8) {
    unsafe { TypedMap::clear(map as *mut TypedMapStringPtr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_string_ptr_drop(map: *mut u8) {
    unsafe { TypedMap::drop_map(map as *mut TypedMapStringPtr) }
}

// ---------------------------------------------------------------------------
// (i64, f64)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_f64_alloc() -> *mut u8 {
    TypedMapI64F64::new() as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_f64_get(map: *mut u8, key: i64, out: *mut f64) -> bool {
    unsafe {
        match TypedMap::get_i64(map as *const TypedMapI64F64, key) {
            Some(v) => {
                *out = v;
                true
            }
            None => false,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_f64_set(map: *mut u8, key: i64, value: f64) {
    unsafe {
        TypedMap::insert_i64(map as *mut TypedMapI64F64, key, value);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_f64_delete(map: *mut u8, key: i64) -> bool {
    unsafe { TypedMap::remove_i64(map as *mut TypedMapI64F64, key).is_some() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_f64_has(map: *mut u8, key: i64) -> bool {
    unsafe { TypedMap::contains_key_i64(map as *const TypedMapI64F64, key) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_f64_len(map: *mut u8) -> u32 {
    unsafe { TypedMap::len(map as *const TypedMapI64F64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_f64_clear(map: *mut u8) {
    unsafe { TypedMap::clear(map as *mut TypedMapI64F64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_f64_drop(map: *mut u8) {
    unsafe { TypedMap::drop_map(map as *mut TypedMapI64F64) }
}

// ---------------------------------------------------------------------------
// (i64, i64)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_i64_alloc() -> *mut u8 {
    TypedMapI64I64::new() as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_i64_get(map: *mut u8, key: i64, out: *mut i64) -> bool {
    unsafe {
        match TypedMap::get_i64(map as *const TypedMapI64I64, key) {
            Some(v) => {
                *out = v;
                true
            }
            None => false,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_i64_set(map: *mut u8, key: i64, value: i64) {
    unsafe {
        TypedMap::insert_i64(map as *mut TypedMapI64I64, key, value);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_i64_delete(map: *mut u8, key: i64) -> bool {
    unsafe { TypedMap::remove_i64(map as *mut TypedMapI64I64, key).is_some() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_i64_has(map: *mut u8, key: i64) -> bool {
    unsafe { TypedMap::contains_key_i64(map as *const TypedMapI64I64, key) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_i64_len(map: *mut u8) -> u32 {
    unsafe { TypedMap::len(map as *const TypedMapI64I64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_i64_clear(map: *mut u8) {
    unsafe { TypedMap::clear(map as *mut TypedMapI64I64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_i64_drop(map: *mut u8) {
    unsafe { TypedMap::drop_map(map as *mut TypedMapI64I64) }
}

// ---------------------------------------------------------------------------
// (i64, ptr)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_ptr_alloc() -> *mut u8 {
    TypedMapI64Ptr::new() as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_ptr_get(
    map: *mut u8,
    key: i64,
    out: *mut *const u8,
) -> bool {
    unsafe {
        match TypedMap::get_i64(map as *const TypedMapI64Ptr, key) {
            Some(v) => {
                *out = v;
                true
            }
            None => false,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_ptr_set(map: *mut u8, key: i64, value: *const u8) {
    unsafe {
        TypedMap::insert_i64(map as *mut TypedMapI64Ptr, key, value);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_ptr_delete(map: *mut u8, key: i64) -> bool {
    unsafe { TypedMap::remove_i64(map as *mut TypedMapI64Ptr, key).is_some() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_ptr_has(map: *mut u8, key: i64) -> bool {
    unsafe { TypedMap::contains_key_i64(map as *const TypedMapI64Ptr, key) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_ptr_len(map: *mut u8) -> u32 {
    unsafe { TypedMap::len(map as *const TypedMapI64Ptr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_ptr_clear(map: *mut u8) {
    unsafe { TypedMap::clear(map as *mut TypedMapI64Ptr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn typed_map_i64_ptr_drop(map: *mut u8) {
    unsafe { TypedMap::drop_map(map as *mut TypedMapI64Ptr) }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> (*const u8, u32) {
        (s.as_ptr(), s.len() as u32)
    }

    // ----- (string, f64) -----

    #[test]
    fn ops_string_f64_round_trip() {
        unsafe {
            let m = typed_map_string_f64_alloc();

            let (k, kl) = key("alpha");
            typed_map_string_f64_set(m, k, kl, 3.14);

            let mut out: f64 = 0.0;
            assert!(typed_map_string_f64_get(m, k, kl, &mut out));
            assert_eq!(out, 3.14);

            assert!(typed_map_string_f64_has(m, k, kl));
            assert_eq!(typed_map_string_f64_len(m), 1);

            typed_map_string_f64_drop(m);
        }
    }

    #[test]
    fn ops_string_f64_multiple_entries() {
        unsafe {
            let m = typed_map_string_f64_alloc();
            let owned: Vec<String> = (0..100).map(|i| format!("k_{i}")).collect();
            for (i, k) in owned.iter().enumerate() {
                typed_map_string_f64_set(m, k.as_ptr(), k.len() as u32, i as f64 * 0.5);
            }
            assert_eq!(typed_map_string_f64_len(m), 100);
            for (i, k) in owned.iter().enumerate() {
                let mut out = 0.0;
                assert!(typed_map_string_f64_get(m, k.as_ptr(), k.len() as u32, &mut out));
                assert_eq!(out, i as f64 * 0.5);
            }
            typed_map_string_f64_drop(m);
        }
    }

    #[test]
    fn ops_string_f64_collision_handling() {
        // Use keys that share characters and lengths to exercise probing.
        unsafe {
            let m = typed_map_string_f64_alloc();
            let keys = ["aa", "ab", "ba", "bb", "ca", "cb", "da", "db"];
            for (i, k) in keys.iter().enumerate() {
                typed_map_string_f64_set(m, k.as_ptr(), k.len() as u32, i as f64);
            }
            for (i, k) in keys.iter().enumerate() {
                let mut out = 0.0;
                assert!(typed_map_string_f64_get(m, k.as_ptr(), k.len() as u32, &mut out));
                assert_eq!(out, i as f64);
            }
            typed_map_string_f64_drop(m);
        }
    }

    #[test]
    fn ops_string_f64_delete_then_missing() {
        unsafe {
            let m = typed_map_string_f64_alloc();
            let (k, kl) = key("gone");
            typed_map_string_f64_set(m, k, kl, 1.0);
            assert!(typed_map_string_f64_delete(m, k, kl));
            let mut out = 0.0;
            assert!(!typed_map_string_f64_get(m, k, kl, &mut out));
            assert!(!typed_map_string_f64_has(m, k, kl));
            // Deleting again is a no-op.
            assert!(!typed_map_string_f64_delete(m, k, kl));
            typed_map_string_f64_drop(m);
        }
    }

    #[test]
    fn ops_string_f64_len_progression() {
        unsafe {
            let m = typed_map_string_f64_alloc();
            assert_eq!(typed_map_string_f64_len(m), 0);
            for i in 0..5 {
                let s = format!("k{i}");
                typed_map_string_f64_set(m, s.as_ptr(), s.len() as u32, i as f64);
            }
            assert_eq!(typed_map_string_f64_len(m), 5);
            // Replace — len stays 5
            let s = "k2".to_string();
            typed_map_string_f64_set(m, s.as_ptr(), s.len() as u32, 999.0);
            assert_eq!(typed_map_string_f64_len(m), 5);
            // Delete one — len drops to 4
            let s = "k0".to_string();
            assert!(typed_map_string_f64_delete(m, s.as_ptr(), s.len() as u32));
            assert_eq!(typed_map_string_f64_len(m), 4);
            typed_map_string_f64_drop(m);
        }
    }

    #[test]
    fn ops_string_f64_clear() {
        unsafe {
            let m = typed_map_string_f64_alloc();
            for i in 0..5 {
                let s = format!("k{i}");
                typed_map_string_f64_set(m, s.as_ptr(), s.len() as u32, i as f64);
            }
            typed_map_string_f64_clear(m);
            assert_eq!(typed_map_string_f64_len(m), 0);
            typed_map_string_f64_drop(m);
        }
    }

    // ----- (string, i64) -----

    #[test]
    fn ops_string_i64_round_trip() {
        unsafe {
            let m = typed_map_string_i64_alloc();
            let (k, kl) = key("count");
            typed_map_string_i64_set(m, k, kl, 42);
            let mut out: i64 = 0;
            assert!(typed_map_string_i64_get(m, k, kl, &mut out));
            assert_eq!(out, 42);
            typed_map_string_i64_drop(m);
        }
    }

    // ----- (string, bool) -----

    #[test]
    fn ops_string_bool_round_trip() {
        unsafe {
            let m = typed_map_string_bool_alloc();
            let (k1, k1l) = key("yes");
            let (k2, k2l) = key("no");
            typed_map_string_bool_set(m, k1, k1l, true);
            typed_map_string_bool_set(m, k2, k2l, false);

            let mut out = false;
            assert!(typed_map_string_bool_get(m, k1, k1l, &mut out));
            assert!(out);
            assert!(typed_map_string_bool_get(m, k2, k2l, &mut out));
            assert!(!out);

            assert_eq!(typed_map_string_bool_len(m), 2);
            typed_map_string_bool_drop(m);
        }
    }

    // ----- (string, ptr) -----

    #[test]
    fn ops_string_ptr_round_trip() {
        unsafe {
            let m = typed_map_string_ptr_alloc();
            let (k, kl) = key("link");
            let payload = StringObj::new("payload");
            typed_map_string_ptr_set(m, k, kl, payload as *const u8);

            let mut out: *const u8 = std::ptr::null();
            assert!(typed_map_string_ptr_get(m, k, kl, &mut out));
            let retrieved = out as *const StringObj;
            assert_eq!(StringObj::as_str(retrieved), "payload");

            StringObj::drop(payload);
            typed_map_string_ptr_drop(m);
        }
    }

    // ----- (i64, f64) -----

    #[test]
    fn ops_i64_f64_round_trip() {
        unsafe {
            let m = typed_map_i64_f64_alloc();
            typed_map_i64_f64_set(m, 7, 1.25);
            let mut out = 0.0;
            assert!(typed_map_i64_f64_get(m, 7, &mut out));
            assert_eq!(out, 1.25);
            assert!(!typed_map_i64_f64_get(m, 99, &mut out));
            typed_map_i64_f64_drop(m);
        }
    }

    #[test]
    fn ops_i64_f64_bulk_100() {
        unsafe {
            let m = typed_map_i64_f64_alloc();
            for i in 0..100i64 {
                typed_map_i64_f64_set(m, i, i as f64 * 0.25);
            }
            assert_eq!(typed_map_i64_f64_len(m), 100);
            for i in 0..100i64 {
                let mut out = 0.0;
                assert!(typed_map_i64_f64_get(m, i, &mut out));
                assert_eq!(out, i as f64 * 0.25);
            }
            typed_map_i64_f64_drop(m);
        }
    }

    #[test]
    fn ops_i64_f64_delete_and_len() {
        unsafe {
            let m = typed_map_i64_f64_alloc();
            for i in 0..5i64 {
                typed_map_i64_f64_set(m, i, i as f64);
            }
            assert_eq!(typed_map_i64_f64_len(m), 5);

            // Replace doesn't change len
            typed_map_i64_f64_set(m, 2, 99.0);
            assert_eq!(typed_map_i64_f64_len(m), 5);

            // Delete one
            assert!(typed_map_i64_f64_delete(m, 0));
            assert_eq!(typed_map_i64_f64_len(m), 4);

            assert!(!typed_map_i64_f64_has(m, 0));
            assert!(typed_map_i64_f64_has(m, 2));

            typed_map_i64_f64_drop(m);
        }
    }

    // ----- (i64, i64) -----

    #[test]
    fn ops_i64_i64_round_trip() {
        unsafe {
            let m = typed_map_i64_i64_alloc();
            typed_map_i64_i64_set(m, -1, 100);
            typed_map_i64_i64_set(m, 0, 200);
            typed_map_i64_i64_set(m, 1, 300);

            let mut out = 0i64;
            assert!(typed_map_i64_i64_get(m, -1, &mut out));
            assert_eq!(out, 100);
            assert!(typed_map_i64_i64_get(m, 0, &mut out));
            assert_eq!(out, 200);
            assert!(typed_map_i64_i64_get(m, 1, &mut out));
            assert_eq!(out, 300);

            typed_map_i64_i64_drop(m);
        }
    }

    // ----- (i64, ptr) -----

    #[test]
    fn ops_i64_ptr_round_trip() {
        unsafe {
            let m = typed_map_i64_ptr_alloc();
            let s1 = StringObj::new("hello");
            let s2 = StringObj::new("world");
            typed_map_i64_ptr_set(m, 1, s1 as *const u8);
            typed_map_i64_ptr_set(m, 2, s2 as *const u8);

            let mut out: *const u8 = std::ptr::null();
            assert!(typed_map_i64_ptr_get(m, 1, &mut out));
            assert_eq!(StringObj::as_str(out as *const StringObj), "hello");
            assert!(typed_map_i64_ptr_get(m, 2, &mut out));
            assert_eq!(StringObj::as_str(out as *const StringObj), "world");

            StringObj::drop(s1);
            StringObj::drop(s2);
            typed_map_i64_ptr_drop(m);
        }
    }
}
