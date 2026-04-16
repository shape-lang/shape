//! Typed hash map for v2 runtime.
//!
//! `TypedMap<K, V>` is a `#[repr(C)]` open-addressing hash table with linear
//! probing. The compiler monomorphizes: `HashMap<string, number>` and
//! `HashMap<string, i32>` are different `TypedMap` instantiations.
//!
//! Concrete type aliases are provided for common instantiations used by FFI/JIT:
//! - `TypedMapStringF64` — `HashMap<string, number>`
//! - `TypedMapStringI64` — `HashMap<string, i64>`
//! - `TypedMapStringPtr` — `HashMap<string, *const u8>`

use super::heap_header::{HeapHeader, HEAP_KIND_V2_TYPED_MAP};
use crate::value_word::ValueWordExt;
use std::alloc::{Layout, alloc_zeroed, dealloc};

/// Sentinel hash values for bucket state.
const HASH_EMPTY: u64 = 0;
const HASH_TOMBSTONE: u64 = 1;

/// Minimum hash value for an occupied bucket (after masking).
const HASH_MIN_OCCUPIED: u64 = 2;

/// Load factor threshold (75%) — grow when `len * 4 >= bucket_count * 3`.
const LOAD_FACTOR_NUM: u32 = 3;
const LOAD_FACTOR_DEN: u32 = 4;

/// Default initial bucket count.
const INITIAL_CAPACITY: u32 = 8;

/// A single bucket in the hash table.
#[repr(C)]
pub struct Bucket<K, V> {
    /// 0 = empty, 1 = tombstone, >= 2 = occupied (stores masked hash).
    pub hash: u64,
    pub key: K,
    pub value: V,
}

/// Typed open-addressing hash map with linear probing.
#[repr(C)]
pub struct TypedMap<K, V> {
    pub header: HeapHeader,
    /// Pointer to bucket array.
    pub buckets: *mut Bucket<K, V>,
    /// Number of buckets (always a power of 2).
    pub bucket_count: u32,
    /// Number of live entries.
    pub len: u32,
    /// Number of tombstones.
    pub tombstone_count: u32,
    pub _pad: u32,
}

// Concrete type aliases for common instantiations.
pub type TypedMapStringF64 = TypedMap<*const u8, f64>;
pub type TypedMapStringI64 = TypedMap<*const u8, i64>;
pub type TypedMapStringPtr = TypedMap<*const u8, *const u8>;

// i64-keyed map aliases.
pub type TypedMapI64F64 = TypedMap<i64, f64>;
pub type TypedMapI64I64 = TypedMap<i64, i64>;
pub type TypedMapI64Ptr = TypedMap<i64, *const u8>;

/// FNV-1a hash for byte slices. Simple, fast, good distribution for short keys.
#[inline]
fn fnv1a_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Ensure a hash value is >= HASH_MIN_OCCUPIED (so it can't be confused with
/// empty/tombstone sentinels).
#[inline]
fn fix_hash(h: u64) -> u64 {
    if h < HASH_MIN_OCCUPIED {
        h.wrapping_add(HASH_MIN_OCCUPIED)
    } else {
        h
    }
}

impl<K: Copy, V: Copy> TypedMap<K, V> {
    /// Allocate a new empty TypedMap.
    pub fn new() -> *mut Self {
        let layout = Layout::new::<Self>();
        let ptr = unsafe { alloc_zeroed(layout) as *mut Self };
        assert!(!ptr.is_null(), "allocation failed for TypedMap");

        unsafe {
            (*ptr).header = HeapHeader::new(HEAP_KIND_V2_TYPED_MAP);
            (*ptr).buckets = std::ptr::null_mut();
            (*ptr).bucket_count = 0;
            (*ptr).len = 0;
            (*ptr).tombstone_count = 0;
            (*ptr)._pad = 0;
        }
        ptr
    }

    /// Number of entries.
    ///
    /// # Safety
    /// `this` must point to a valid `TypedMap`.
    #[inline]
    pub unsafe fn len(this: *const Self) -> u32 {
        unsafe { (*this).len }
    }

    /// Whether the map is empty.
    ///
    /// # Safety
    /// `this` must point to a valid `TypedMap`.
    #[inline]
    pub unsafe fn is_empty(this: *const Self) -> bool {
        unsafe { (*this).len == 0 }
    }

    /// Deallocate the map and its bucket array.
    ///
    /// # Safety
    /// `ptr` must point to a `TypedMap` allocated by this module.
    pub unsafe fn drop_map(ptr: *mut Self) {
        unsafe {
            let map = &*ptr;
            if map.bucket_count > 0 && !map.buckets.is_null() {
                let bucket_layout = Layout::array::<Bucket<K, V>>(map.bucket_count as usize)
                    .expect("invalid bucket layout");
                dealloc(map.buckets as *mut u8, bucket_layout);
            }
            let layout = Layout::new::<Self>();
            dealloc(ptr as *mut u8, layout);
        }
    }

    /// Allocate a zeroed bucket array. All buckets have hash = 0 (empty).
    fn alloc_buckets(count: u32) -> *mut Bucket<K, V> {
        let layout =
            Layout::array::<Bucket<K, V>>(count as usize).expect("invalid bucket layout");
        let ptr = unsafe { alloc_zeroed(layout) as *mut Bucket<K, V> };
        assert!(!ptr.is_null(), "allocation failed for TypedMap buckets");
        ptr
    }

    /// Ensure the table has room for at least one more entry, growing if needed.
    ///
    /// # Safety
    /// `this` must point to a valid `TypedMap`.
    unsafe fn ensure_capacity(this: *mut Self) {
        unsafe {
            let map = &*this;
            if map.bucket_count == 0 {
                (*this).buckets = Self::alloc_buckets(INITIAL_CAPACITY);
                (*this).bucket_count = INITIAL_CAPACITY;
                return;
            }
            // Grow when (len + tombstones) * 4 >= bucket_count * 3
            let used = map.len + map.tombstone_count;
            if used * LOAD_FACTOR_DEN >= map.bucket_count * LOAD_FACTOR_NUM {
                Self::grow(this);
            }
        }
    }

    /// Double the bucket array and rehash all live entries.
    ///
    /// # Safety
    /// `this` must point to a valid `TypedMap` with bucket_count > 0.
    unsafe fn grow(this: *mut Self) {
        unsafe {
            let map = &*this;
            let old_buckets = map.buckets;
            let old_count = map.bucket_count;

            let new_count = old_count * 2;
            let new_buckets = Self::alloc_buckets(new_count);
            let mask = new_count - 1; // power-of-2 mask

            // Rehash live entries
            for i in 0..old_count {
                let bucket = &*old_buckets.add(i as usize);
                if bucket.hash >= HASH_MIN_OCCUPIED {
                    let mut idx = (bucket.hash as u32) & mask;
                    loop {
                        let dst = &*new_buckets.add(idx as usize);
                        if dst.hash == HASH_EMPTY {
                            std::ptr::write(
                                new_buckets.add(idx as usize),
                                Bucket {
                                    hash: bucket.hash,
                                    key: bucket.key,
                                    value: bucket.value,
                                },
                            );
                            break;
                        }
                        idx = (idx + 1) & mask;
                    }
                }
            }

            // Free old buckets
            let old_layout = Layout::array::<Bucket<K, V>>(old_count as usize)
                .expect("invalid bucket layout");
            dealloc(old_buckets as *mut u8, old_layout);

            (*this).buckets = new_buckets;
            (*this).bucket_count = new_count;
            (*this).tombstone_count = 0;
        }
    }
}

// --- String-keyed map operations ---
//
// These operate on TypedMap<*const u8, V> where keys are `*const StringObj`.
// We compare by string content, not pointer identity.

use super::string_obj::StringObj;

impl<V: Copy> TypedMap<*const u8, V> {
    /// Insert a key-value pair. If the key already exists, updates the value
    /// and returns the old value.
    ///
    /// The map retains the key pointer (caller must ensure it stays alive via
    /// refcounting). If the key already exists, the existing key pointer is kept.
    ///
    /// # Safety
    /// `this` must point to a valid `TypedMap`. `key` must point to a valid `StringObj`.
    pub unsafe fn insert(this: *mut Self, key: *const StringObj, value: V) -> Option<V> {
        unsafe {
            Self::ensure_capacity(this);

            let key_str = StringObj::as_str(key);
            let hash = fix_hash(fnv1a_hash(key_str.as_bytes()));
            let map = &*this;
            let mask = map.bucket_count - 1;
            let mut idx = (hash as u32) & mask;
            let mut first_tombstone: Option<u32> = None;

            loop {
                let bucket = &*map.buckets.add(idx as usize);
                match bucket.hash {
                    HASH_EMPTY => {
                        // Key not found — insert at first tombstone if we saw one, else here.
                        let insert_idx = first_tombstone.unwrap_or(idx);
                        if first_tombstone.is_some() {
                            (*this).tombstone_count -= 1;
                        }
                        std::ptr::write(
                            (*this).buckets.add(insert_idx as usize),
                            Bucket {
                                hash,
                                key: key as *const u8,
                                value,
                            },
                        );
                        (*this).len += 1;
                        return None;
                    }
                    HASH_TOMBSTONE => {
                        if first_tombstone.is_none() {
                            first_tombstone = Some(idx);
                        }
                    }
                    h if h == hash => {
                        // Hash matches — compare key content.
                        let existing_key = bucket.key as *const StringObj;
                        let existing_str = StringObj::as_str(existing_key);
                        if existing_str == key_str {
                            // Key exists — update value, return old.
                            let old = bucket.value;
                            (*(*this).buckets.add(idx as usize)).value = value;
                            return Some(old);
                        }
                    }
                    _ => {}
                }
                idx = (idx + 1) & mask;
            }
        }
    }

    /// Look up a value by string key.
    ///
    /// # Safety
    /// `this` must point to a valid `TypedMap`. `key` must point to a valid `StringObj`.
    pub unsafe fn get(this: *const Self, key: *const StringObj) -> Option<V> {
        unsafe {
            let map = &*this;
            if map.len == 0 || map.bucket_count == 0 {
                return None;
            }

            let key_str = StringObj::as_str(key);
            let hash = fix_hash(fnv1a_hash(key_str.as_bytes()));
            let mask = map.bucket_count - 1;
            let mut idx = (hash as u32) & mask;

            loop {
                let bucket = &*map.buckets.add(idx as usize);
                match bucket.hash {
                    HASH_EMPTY => return None,
                    HASH_TOMBSTONE => {}
                    h if h == hash => {
                        let existing_key = bucket.key as *const StringObj;
                        let existing_str = StringObj::as_str(existing_key);
                        if existing_str == key_str {
                            return Some(bucket.value);
                        }
                    }
                    _ => {}
                }
                idx = (idx + 1) & mask;
            }
        }
    }

    /// Check if the map contains a key.
    ///
    /// # Safety
    /// `this` must point to a valid `TypedMap`. `key` must point to a valid `StringObj`.
    pub unsafe fn contains_key(this: *const Self, key: *const StringObj) -> bool {
        unsafe { Self::get(this, key).is_some() }
    }

    /// Remove a key from the map, returning the value if it was present.
    ///
    /// # Safety
    /// `this` must point to a valid `TypedMap`. `key` must point to a valid `StringObj`.
    pub unsafe fn remove(this: *mut Self, key: *const StringObj) -> Option<V> {
        unsafe {
            let map = &*this;
            if map.len == 0 || map.bucket_count == 0 {
                return None;
            }

            let key_str = StringObj::as_str(key);
            let hash = fix_hash(fnv1a_hash(key_str.as_bytes()));
            let mask = map.bucket_count - 1;
            let mut idx = (hash as u32) & mask;

            loop {
                let bucket = &*map.buckets.add(idx as usize);
                match bucket.hash {
                    HASH_EMPTY => return None,
                    HASH_TOMBSTONE => {}
                    h if h == hash => {
                        let existing_key = bucket.key as *const StringObj;
                        let existing_str = StringObj::as_str(existing_key);
                        if existing_str == key_str {
                            let old_value = bucket.value;
                            // Replace with tombstone
                            (*(*this).buckets.add(idx as usize)).hash = HASH_TOMBSTONE;
                            (*this).len -= 1;
                            (*this).tombstone_count += 1;
                            return Some(old_value);
                        }
                    }
                    _ => {}
                }
                idx = (idx + 1) & mask;
            }
        }
    }
}

// --- i64-keyed map operations ---
//
// These operate on TypedMap<i64, V>. Keys are compared by raw integer
// equality. The hash mixes the i64 with FNV-1a over its bytes for a decent
// distribution across small integers.

impl<V: Copy> TypedMap<i64, V> {
    /// Insert a key-value pair. If the key already exists, updates the value
    /// and returns the old value.
    ///
    /// # Safety
    /// `this` must point to a valid `TypedMap<i64, V>`.
    pub unsafe fn insert_i64(this: *mut Self, key: i64, value: V) -> Option<V> {
        unsafe {
            Self::ensure_capacity(this);

            let hash = fix_hash(fnv1a_hash(&key.to_le_bytes()));
            let map = &*this;
            let mask = map.bucket_count - 1;
            let mut idx = (hash as u32) & mask;
            let mut first_tombstone: Option<u32> = None;

            loop {
                let bucket = &*map.buckets.add(idx as usize);
                match bucket.hash {
                    HASH_EMPTY => {
                        let insert_idx = first_tombstone.unwrap_or(idx);
                        if first_tombstone.is_some() {
                            (*this).tombstone_count -= 1;
                        }
                        std::ptr::write(
                            (*this).buckets.add(insert_idx as usize),
                            Bucket { hash, key, value },
                        );
                        (*this).len += 1;
                        return None;
                    }
                    HASH_TOMBSTONE => {
                        if first_tombstone.is_none() {
                            first_tombstone = Some(idx);
                        }
                    }
                    h if h == hash => {
                        if bucket.key == key {
                            let old = bucket.value;
                            (*(*this).buckets.add(idx as usize)).value = value;
                            return Some(old);
                        }
                    }
                    _ => {}
                }
                idx = (idx + 1) & mask;
            }
        }
    }

    /// Look up a value by i64 key.
    ///
    /// # Safety
    /// `this` must point to a valid `TypedMap<i64, V>`.
    pub unsafe fn get_i64(this: *const Self, key: i64) -> Option<V> {
        unsafe {
            let map = &*this;
            if map.len == 0 || map.bucket_count == 0 {
                return None;
            }

            let hash = fix_hash(fnv1a_hash(&key.to_le_bytes()));
            let mask = map.bucket_count - 1;
            let mut idx = (hash as u32) & mask;

            loop {
                let bucket = &*map.buckets.add(idx as usize);
                match bucket.hash {
                    HASH_EMPTY => return None,
                    HASH_TOMBSTONE => {}
                    h if h == hash => {
                        if bucket.key == key {
                            return Some(bucket.value);
                        }
                    }
                    _ => {}
                }
                idx = (idx + 1) & mask;
            }
        }
    }

    /// Check if the map contains an i64 key.
    ///
    /// # Safety
    /// `this` must point to a valid `TypedMap<i64, V>`.
    pub unsafe fn contains_key_i64(this: *const Self, key: i64) -> bool {
        unsafe { Self::get_i64(this, key).is_some() }
    }

    /// Remove an i64 key from the map, returning the value if it was present.
    ///
    /// # Safety
    /// `this` must point to a valid `TypedMap<i64, V>`.
    pub unsafe fn remove_i64(this: *mut Self, key: i64) -> Option<V> {
        unsafe {
            let map = &*this;
            if map.len == 0 || map.bucket_count == 0 {
                return None;
            }

            let hash = fix_hash(fnv1a_hash(&key.to_le_bytes()));
            let mask = map.bucket_count - 1;
            let mut idx = (hash as u32) & mask;

            loop {
                let bucket = &*map.buckets.add(idx as usize);
                match bucket.hash {
                    HASH_EMPTY => return None,
                    HASH_TOMBSTONE => {}
                    h if h == hash => {
                        if bucket.key == key {
                            let old_value = bucket.value;
                            (*(*this).buckets.add(idx as usize)).hash = HASH_TOMBSTONE;
                            (*this).len -= 1;
                            (*this).tombstone_count += 1;
                            return Some(old_value);
                        }
                    }
                    _ => {}
                }
                idx = (idx + 1) & mask;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a StringObj from a &str for testing.
    fn make_key(s: &str) -> *mut StringObj {
        StringObj::new(s)
    }

    #[test]
    fn test_new_empty_map() {
        let map = TypedMapStringF64::new();
        unsafe {
            assert_eq!(TypedMap::len(map), 0);
            assert!(TypedMap::is_empty(map));
            assert_eq!((*map).header.kind(), HEAP_KIND_V2_TYPED_MAP);
            assert_eq!((*map).header.get_refcount(), 1);
            TypedMap::drop_map(map);
        }
    }

    #[test]
    fn test_insert_and_get() {
        let map = TypedMapStringF64::new();
        let k1 = make_key("x");
        let k2 = make_key("y");
        let k3 = make_key("z");

        unsafe {
            assert_eq!(TypedMap::insert(map, k1, 1.0), None);
            assert_eq!(TypedMap::insert(map, k2, 2.0), None);
            assert_eq!(TypedMap::insert(map, k3, 3.0), None);
            assert_eq!(TypedMap::len(map), 3);

            // Look up using fresh key pointers (same content, different allocation)
            let lookup_x = make_key("x");
            let lookup_y = make_key("y");
            let lookup_z = make_key("z");

            assert_eq!(TypedMap::get(map, lookup_x), Some(1.0));
            assert_eq!(TypedMap::get(map, lookup_y), Some(2.0));
            assert_eq!(TypedMap::get(map, lookup_z), Some(3.0));

            let missing = make_key("missing");
            assert_eq!(TypedMap::get(map, missing), None);

            StringObj::drop(lookup_x);
            StringObj::drop(lookup_y);
            StringObj::drop(lookup_z);
            StringObj::drop(missing);
            StringObj::drop(k1);
            StringObj::drop(k2);
            StringObj::drop(k3);
            TypedMap::drop_map(map);
        }
    }

    #[test]
    fn test_insert_update_returns_old_value() {
        let map = TypedMapStringF64::new();
        let k1 = make_key("key");
        let k2 = make_key("key"); // same content

        unsafe {
            assert_eq!(TypedMap::insert(map, k1, 1.0), None);
            assert_eq!(TypedMap::insert(map, k2, 2.0), Some(1.0)); // returns old
            assert_eq!(TypedMap::len(map), 1); // still 1 entry

            let lookup = make_key("key");
            assert_eq!(TypedMap::get(map, lookup), Some(2.0)); // updated value

            StringObj::drop(lookup);
            StringObj::drop(k1);
            StringObj::drop(k2);
            TypedMap::drop_map(map);
        }
    }

    #[test]
    fn test_contains_key() {
        let map = TypedMapStringF64::new();
        let k = make_key("present");

        unsafe {
            TypedMap::insert(map, k, 42.0);

            let lookup_yes = make_key("present");
            let lookup_no = make_key("absent");
            assert!(TypedMap::contains_key(map, lookup_yes));
            assert!(!TypedMap::contains_key(map, lookup_no));

            StringObj::drop(lookup_yes);
            StringObj::drop(lookup_no);
            StringObj::drop(k);
            TypedMap::drop_map(map);
        }
    }

    #[test]
    fn test_remove() {
        let map = TypedMapStringI64::new();
        let k1 = make_key("a");
        let k2 = make_key("b");

        unsafe {
            TypedMap::insert(map, k1, 10i64);
            TypedMap::insert(map, k2, 20i64);
            assert_eq!(TypedMap::len(map), 2);

            let rm_key = make_key("a");
            assert_eq!(TypedMap::remove(map, rm_key), Some(10i64));
            assert_eq!(TypedMap::len(map), 1);

            // Can't find removed key
            let lookup = make_key("a");
            assert_eq!(TypedMap::get(map, lookup), None);

            // Other key still present
            let lookup_b = make_key("b");
            assert_eq!(TypedMap::get(map, lookup_b), Some(20i64));

            // Remove non-existent key returns None
            let rm_missing = make_key("missing");
            assert_eq!(TypedMap::remove(map, rm_missing), None);

            StringObj::drop(rm_key);
            StringObj::drop(lookup);
            StringObj::drop(lookup_b);
            StringObj::drop(rm_missing);
            StringObj::drop(k1);
            StringObj::drop(k2);
            TypedMap::drop_map(map);
        }
    }

    #[test]
    fn test_grow_and_rehash() {
        let map = TypedMapStringF64::new();
        let mut keys = Vec::new();

        unsafe {
            // Insert enough entries to trigger multiple grows.
            for i in 0..50 {
                let key = make_key(&format!("key_{i}"));
                TypedMap::insert(map, key, i as f64);
                keys.push(key);
            }

            assert_eq!(TypedMap::len(map), 50);

            // Verify all entries are still accessible after rehashing.
            for i in 0..50 {
                let lookup = make_key(&format!("key_{i}"));
                assert_eq!(
                    TypedMap::get(map, lookup),
                    Some(i as f64),
                    "missing key_{i} after grow"
                );
                StringObj::drop(lookup);
            }

            for k in keys {
                StringObj::drop(k);
            }
            TypedMap::drop_map(map);
        }
    }

    #[test]
    fn test_collision_handling() {
        // Insert many keys — some will collide due to hash masking.
        let map = TypedMapStringF64::new();
        let mut keys = Vec::new();

        unsafe {
            for i in 0..20 {
                let key = make_key(&format!("{i}"));
                TypedMap::insert(map, key, i as f64);
                keys.push(key);
            }

            assert_eq!(TypedMap::len(map), 20);

            for i in 0..20 {
                let lookup = make_key(&format!("{i}"));
                assert_eq!(TypedMap::get(map, lookup), Some(i as f64));
                StringObj::drop(lookup);
            }

            for k in keys {
                StringObj::drop(k);
            }
            TypedMap::drop_map(map);
        }
    }

    #[test]
    fn test_remove_then_insert_reuses_tombstone() {
        let map = TypedMapStringF64::new();

        unsafe {
            let k1 = make_key("alpha");
            TypedMap::insert(map, k1, 1.0);

            let rm = make_key("alpha");
            TypedMap::remove(map, rm);
            assert_eq!(TypedMap::len(map), 0);
            assert_eq!((*map).tombstone_count, 1);

            // Insert same key again — should reuse tombstone slot.
            let k2 = make_key("alpha");
            TypedMap::insert(map, k2, 2.0);
            assert_eq!(TypedMap::len(map), 1);
            assert_eq!((*map).tombstone_count, 0);

            let lookup = make_key("alpha");
            assert_eq!(TypedMap::get(map, lookup), Some(2.0));

            StringObj::drop(lookup);
            StringObj::drop(k1);
            StringObj::drop(k2);
            StringObj::drop(rm);
            TypedMap::drop_map(map);
        }
    }

    #[test]
    fn test_get_on_empty_map() {
        let map = TypedMapStringF64::new();
        unsafe {
            let k = make_key("anything");
            assert_eq!(TypedMap::get(map, k), None);
            StringObj::drop(k);
            TypedMap::drop_map(map);
        }
    }

    #[test]
    fn test_string_ptr_map() {
        let map = TypedMapStringPtr::new();
        unsafe {
            let k = make_key("ptr_key");
            let val_str = make_key("value");
            TypedMap::insert(map, k, val_str as *const u8);

            let lookup = make_key("ptr_key");
            let result = TypedMap::get(map, lookup);
            assert!(result.is_some());
            let retrieved = result.unwrap() as *const StringObj;
            assert_eq!(StringObj::as_str(retrieved), "value");

            StringObj::drop(lookup);
            StringObj::drop(k);
            StringObj::drop(val_str);
            TypedMap::drop_map(map);
        }
    }
}
