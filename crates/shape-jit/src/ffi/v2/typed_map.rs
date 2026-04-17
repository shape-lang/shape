//! Typed HashMap FFI helpers for v2 JIT codegen.
//!
//! These functions operate directly on `ValueWord`-encoded `u64` bits whose
//! heap variant is `HeapValue::HashMap(Box<HashMapData>)`. They let JIT code
//! call `map.get(key)`, `map.has(key)`, `map.set(key, value)`, and `map.len()`
//! without going through the generic method-dispatch trampoline.
//!
//! ## Semantics parity with the interpreter
//!
//! These helpers mirror the behaviour of `op_map_get_str_i64`,
//! `op_map_get_str_f64`, `op_map_has_str`, `op_map_set_str_i64`, and
//! `op_map_len_typed` in
//! `crates/shape-vm/src/executor/objects/typed_access.rs`. Edge cases
//! (missing key → none, empty map, non-string keys, null maps) resolve
//! identically.
//!
//! ## Safety & null handling
//!
//! Every helper tolerates a null / non-HashMap `map_bits` and returns a
//! conservative fallback (`none`/`false`/`0`) rather than crashing. This
//! matches the interpreter's tolerance of flow-sensitive type narrowing and
//! keeps the JIT hot-path free of deopt branches.

use shape_value::heap_value::HashMapData;
use shape_value::{ValueWord, ValueWordExt};

/// Return the raw u64 bits for the `none` sentinel.
#[inline(always)]
fn none_bits() -> u64 {
    <u64 as ValueWordExt>::none()
}

/// Scan for the entry whose string key equals `key_str`.
/// Uses the shape (hidden class) fast path when available, otherwise the
/// hash bucket + linear compare fallback.
#[inline]
fn find_str_value_bits(map: &HashMapData, key_str: &str, key_bits: u64) -> Option<u64> {
    // Fast path: if the map has a shape, all keys are strings and the
    // layout is index-based.
    if let Some(v) = map.shape_get(key_str) {
        return Some(*v);
    }
    let hash = key_bits.vw_hash();
    let bucket = map.index.get(&hash)?;
    for &idx in bucket {
        if let Some(k) = map.keys[idx].as_str() {
            if k == key_str {
                return Some(map.values[idx]);
            }
        }
    }
    None
}

/// Get value from `HashMap<string, int>` by string key.
///
/// Returns the value's raw `ValueWord` bits (an integer `ValueWord`), or
/// `none_bits()` if:
/// - the map is null / not a HashMap
/// - the key is not a string
/// - the key is not present
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_map_get_str_i64(map_bits: u64, key_bits: u64) -> u64 {
    let Some(map) = map_bits.as_hashmap_data() else {
        return none_bits();
    };
    let Some(key_str) = key_bits.as_str() else {
        return none_bits();
    };
    find_str_value_bits(map, key_str, key_bits).unwrap_or_else(none_bits)
}

/// Get value from `HashMap<string, float>` by string key.
///
/// Returns the stored float as a native `f64` for direct use in JIT-compiled
/// arithmetic. Missing-key / type-mismatch cases return `0.0` — callers that
/// need to distinguish absence from a zero value should use
/// `jit_v2_map_has_str` first.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_map_get_str_f64(map_bits: u64, key_bits: u64) -> f64 {
    let Some(map) = map_bits.as_hashmap_data() else {
        return 0.0;
    };
    let Some(key_str) = key_bits.as_str() else {
        return 0.0;
    };
    match find_str_value_bits(map, key_str, key_bits) {
        Some(bits) => bits.to_number().unwrap_or(0.0),
        None => 0.0,
    }
}

/// Check whether a string key exists in a HashMap.
///
/// Returns `1` if present, `0` otherwise. Null / non-HashMap / non-string
/// inputs all return `0`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_map_has_str(map_bits: u64, key_bits: u64) -> u64 {
    let Some(map) = map_bits.as_hashmap_data() else {
        return 0;
    };
    let Some(key_str) = key_bits.as_str() else {
        return 0;
    };
    find_str_value_bits(map, key_str, key_bits).is_some() as u64
}

/// Set a string-keyed, int-valued entry in a HashMap.
///
/// The underlying `HeapValue::HashMap` is mutated in-place via
/// `as_hashmap_mut`, which performs copy-on-write when the `Arc` is shared.
/// The returned bits are the (possibly new) map handle to store back into the
/// receiver slot.
///
/// Returns `map_bits` unchanged on any error (null map, non-string key).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_map_set_str_i64(map_bits: u64, key_bits: u64, value_bits: u64) -> u64 {
    // Bail out early on null / non-HashMap receivers or non-string keys.
    if map_bits.as_hashmap_data().is_none() {
        return map_bits;
    }
    if key_bits.as_str().is_none() {
        return map_bits;
    }

    // Materialise a ValueWord from the raw bits. This bumps the Arc refcount
    // so the subsequent `as_hashmap_mut` sees a definite owner count and the
    // CoW clone only fires when someone else also holds a reference.
    let mut map_vw = unsafe { ValueWord::clone_from_bits(map_bits) };
    let key = unsafe { ValueWord::clone_from_bits(key_bits) };
    let value = unsafe { ValueWord::clone_from_bits(value_bits) };

    let Some(map_data) = map_vw.as_hashmap_mut() else {
        // Shouldn't happen after the guard above, but stay defensive.
        return map_bits;
    };

    let hash = key.vw_hash();

    // Overwrite existing entry if the key is already present.
    if let Some(bucket) = map_data.index.get(&hash).cloned() {
        for idx in bucket {
            if map_data.keys[idx].vw_equals(&key) {
                map_data.keys[idx] = key;
                map_data.values[idx] = value;
                return map_vw.into_raw_bits();
            }
        }
    }

    // Insert new entry and keep the shape (hidden class) in sync.
    let new_idx = map_data.keys.len();
    if let Some(shape_id) = map_data.shape_id {
        if let Some(ks) = key.as_str() {
            let prop_hash = shape_value::hash_property_name(ks);
            map_data.shape_id = shape_value::shape_transition(shape_id, prop_hash);
        } else {
            map_data.shape_id = None;
        }
    }
    map_data.keys.push(key);
    map_data.values.push(value);
    map_data.index.entry(hash).or_default().push(new_idx);

    map_vw.into_raw_bits()
}

/// Return the number of entries in a HashMap as a raw `i64`.
///
/// Returns `0` for null / non-HashMap inputs.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_map_len(map_bits: u64) -> i64 {
    match map_bits.as_hashmap_data() {
        Some(map) => map.keys.len() as i64,
        None => 0,
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Tests
// ════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::heap_value::{HashMapData, HeapValue};
    use shape_value::{ValueWord, ValueWordExt};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn str_vw(s: &str) -> ValueWord {
        ValueWord::from_string(Arc::new(s.to_string()))
    }

    /// Build a `HashMap<string, int>` ValueWord bits from a slice of entries.
    /// Intentionally does NOT compute a shape — exercises the hash-bucket
    /// fallback path.
    fn make_str_int_map_no_shape(entries: &[(&str, i64)]) -> u64 {
        let mut data = HashMapData {
            keys: Vec::new(),
            values: Vec::new(),
            index: HashMap::new(),
            shape_id: None,
        };
        for (k, v) in entries {
            let key = str_vw(k);
            let idx = data.keys.len();
            data.index.entry(key.vw_hash()).or_default().push(idx);
            data.keys.push(key);
            data.values.push(ValueWord::from_i64(*v));
        }
        ValueWord::from_heap_value(HeapValue::HashMap(Box::new(data)))
    }

    fn make_str_f64_map_no_shape(entries: &[(&str, f64)]) -> u64 {
        let mut data = HashMapData {
            keys: Vec::new(),
            values: Vec::new(),
            index: HashMap::new(),
            shape_id: None,
        };
        for (k, v) in entries {
            let key = str_vw(k);
            let idx = data.keys.len();
            data.index.entry(key.vw_hash()).or_default().push(idx);
            data.keys.push(key);
            data.values.push(ValueWord::from_f64(*v));
        }
        ValueWord::from_heap_value(HeapValue::HashMap(Box::new(data)))
    }

    // ── get_str_i64 ─────────────────────────────────────────────────────

    #[test]
    fn get_str_i64_found() {
        let map = make_str_int_map_no_shape(&[("x", 42), ("y", 99)]);
        let key = str_vw("x");
        let got = jit_v2_map_get_str_i64(map, key);
        assert_eq!(got.to_number(), Some(42.0));
    }

    #[test]
    fn get_str_i64_missing_returns_none() {
        let map = make_str_int_map_no_shape(&[("x", 42)]);
        let key = str_vw("nope");
        assert_eq!(jit_v2_map_get_str_i64(map, key), none_bits());
    }

    #[test]
    fn get_str_i64_empty_map_returns_none() {
        let map = make_str_int_map_no_shape(&[]);
        let key = str_vw("anything");
        assert_eq!(jit_v2_map_get_str_i64(map, key), none_bits());
    }

    #[test]
    fn get_str_i64_null_map_returns_none() {
        // 0 is a definitely-not-a-heap-pointer bit pattern.
        let key = str_vw("x");
        assert_eq!(jit_v2_map_get_str_i64(0, key), none_bits());
    }

    #[test]
    fn get_str_i64_non_string_key_returns_none() {
        let map = make_str_int_map_no_shape(&[("x", 42)]);
        let key = ValueWord::from_i64(123); // not a string
        assert_eq!(jit_v2_map_get_str_i64(map, key), none_bits());
    }

    #[test]
    fn get_str_i64_multiple_entries_distinct_keys() {
        let map = make_str_int_map_no_shape(&[("a", 1), ("b", 2), ("c", 3), ("d", 4)]);
        assert_eq!(
            jit_v2_map_get_str_i64(map, str_vw("a")).to_number(),
            Some(1.0)
        );
        assert_eq!(
            jit_v2_map_get_str_i64(map, str_vw("b")).to_number(),
            Some(2.0)
        );
        assert_eq!(
            jit_v2_map_get_str_i64(map, str_vw("c")).to_number(),
            Some(3.0)
        );
        assert_eq!(
            jit_v2_map_get_str_i64(map, str_vw("d")).to_number(),
            Some(4.0)
        );
    }

    // ── get_str_f64 ─────────────────────────────────────────────────────

    #[test]
    fn get_str_f64_found() {
        let map = make_str_f64_map_no_shape(&[("pi", 3.14)]);
        let key = str_vw("pi");
        let got = jit_v2_map_get_str_f64(map, key);
        assert!((got - 3.14).abs() < 1e-12);
    }

    #[test]
    fn get_str_f64_missing_returns_zero() {
        let map = make_str_f64_map_no_shape(&[("pi", 3.14)]);
        let key = str_vw("e");
        assert_eq!(jit_v2_map_get_str_f64(map, key), 0.0);
    }

    #[test]
    fn get_str_f64_null_map_returns_zero() {
        let key = str_vw("x");
        assert_eq!(jit_v2_map_get_str_f64(0, key), 0.0);
    }

    // ── has_str ─────────────────────────────────────────────────────────

    #[test]
    fn has_str_true() {
        let map = make_str_int_map_no_shape(&[("key", 1)]);
        let key = str_vw("key");
        assert_eq!(jit_v2_map_has_str(map, key), 1);
    }

    #[test]
    fn has_str_false_on_missing() {
        let map = make_str_int_map_no_shape(&[("other", 1)]);
        let key = str_vw("key");
        assert_eq!(jit_v2_map_has_str(map, key), 0);
    }

    #[test]
    fn has_str_false_on_empty_map() {
        let map = make_str_int_map_no_shape(&[]);
        let key = str_vw("anything");
        assert_eq!(jit_v2_map_has_str(map, key), 0);
    }

    #[test]
    fn has_str_false_on_null_map() {
        let key = str_vw("x");
        assert_eq!(jit_v2_map_has_str(0, key), 0);
    }

    // ── map_len ─────────────────────────────────────────────────────────

    #[test]
    fn map_len_counts_entries() {
        let map = make_str_int_map_no_shape(&[("a", 1), ("b", 2), ("c", 3)]);
        assert_eq!(jit_v2_map_len(map), 3);
    }

    #[test]
    fn map_len_empty_is_zero() {
        let map = make_str_int_map_no_shape(&[]);
        assert_eq!(jit_v2_map_len(map), 0);
    }

    #[test]
    fn map_len_null_is_zero() {
        assert_eq!(jit_v2_map_len(0), 0);
    }

    // ── set_str_i64 ─────────────────────────────────────────────────────

    #[test]
    fn set_str_i64_inserts_new_key() {
        let map = make_str_int_map_no_shape(&[]);
        let key = str_vw("k");
        let val = ValueWord::from_i64(777);
        let new_map = jit_v2_map_set_str_i64(map, key, val);

        assert_eq!(jit_v2_map_len(new_map), 1);
        let fetched = jit_v2_map_get_str_i64(new_map, str_vw("k"));
        assert_eq!(fetched.to_number(), Some(777.0));
    }

    #[test]
    fn set_str_i64_overwrites_existing_key() {
        let map = make_str_int_map_no_shape(&[("k", 1)]);
        let key = str_vw("k");
        let val = ValueWord::from_i64(2);
        let new_map = jit_v2_map_set_str_i64(map, key, val);

        assert_eq!(jit_v2_map_len(new_map), 1);
        let fetched = jit_v2_map_get_str_i64(new_map, str_vw("k"));
        assert_eq!(fetched.to_number(), Some(2.0));
    }

    #[test]
    fn set_str_i64_preserves_other_entries() {
        let map = make_str_int_map_no_shape(&[("a", 1), ("b", 2)]);
        let new_map = jit_v2_map_set_str_i64(map, str_vw("c"), ValueWord::from_i64(3));

        assert_eq!(jit_v2_map_len(new_map), 3);
        assert_eq!(
            jit_v2_map_get_str_i64(new_map, str_vw("a")).to_number(),
            Some(1.0)
        );
        assert_eq!(
            jit_v2_map_get_str_i64(new_map, str_vw("b")).to_number(),
            Some(2.0)
        );
        assert_eq!(
            jit_v2_map_get_str_i64(new_map, str_vw("c")).to_number(),
            Some(3.0)
        );
    }

    #[test]
    fn set_str_i64_non_string_key_is_noop() {
        let map = make_str_int_map_no_shape(&[("a", 1)]);
        let non_str_key = ValueWord::from_i64(7);
        let returned = jit_v2_map_set_str_i64(map, non_str_key, ValueWord::from_i64(99));

        assert_eq!(returned, map);
        assert_eq!(jit_v2_map_len(returned), 1);
    }

    #[test]
    fn set_str_i64_null_map_is_noop() {
        let returned = jit_v2_map_set_str_i64(0, str_vw("k"), ValueWord::from_i64(1));
        assert_eq!(returned, 0);
    }

    // ── Shape fast-path parity ─────────────────────────────────────────

    #[test]
    fn get_str_i64_respects_shape_fast_path() {
        // Use the ValueWord constructor that computes a shape to exercise
        // the shape_get fast path. With 2 string keys, HashMapData will
        // allocate a shape (ShapeId) and use index-based lookups.
        let keys = vec![str_vw("x"), str_vw("y")];
        let values = vec![ValueWord::from_i64(10), ValueWord::from_i64(20)];
        let map = ValueWord::from_hashmap_pairs(keys, values);
        let got = jit_v2_map_get_str_i64(map, str_vw("y"));
        assert_eq!(got.to_number(), Some(20.0));
    }

    #[test]
    fn has_str_respects_shape_fast_path() {
        let keys = vec![str_vw("a"), str_vw("b")];
        let values = vec![ValueWord::from_i64(1), ValueWord::from_i64(2)];
        let map = ValueWord::from_hashmap_pairs(keys, values);
        assert_eq!(jit_v2_map_has_str(map, str_vw("a")), 1);
        assert_eq!(jit_v2_map_has_str(map, str_vw("missing")), 0);
    }

    // ── Hash-collision exercise ─────────────────────────────────────────

    #[test]
    fn get_str_i64_same_hash_different_keys() {
        // Two keys with the same hash intentionally force the bucket-scan
        // linear compare. Build such a bucket by hand and verify we return
        // the right value for each key (not the first one in the bucket).
        let mut data = HashMapData {
            keys: Vec::new(),
            values: Vec::new(),
            index: HashMap::new(),
            shape_id: None,
        };
        let k1 = str_vw("alpha");
        let k2 = str_vw("beta");
        // Share one bucket: use k1's real hash for both entries.
        let shared_hash = k1.vw_hash();
        data.keys.push(k1);
        data.values.push(ValueWord::from_i64(111));
        data.keys.push(k2);
        data.values.push(ValueWord::from_i64(222));
        data.index.insert(shared_hash, vec![0, 1]);
        // k2's real hash differs from shared_hash, so a lookup on "beta"
        // would miss the shared bucket; only insert under its own hash too
        // if we want `get("beta")` to succeed via the normal hash path.
        let k2_probe = str_vw("beta");
        if k2_probe.vw_hash() != shared_hash {
            data.index.insert(k2_probe.vw_hash(), vec![1]);
        }
        let map = ValueWord::from_heap_value(HeapValue::HashMap(Box::new(data)));

        // Lookup "alpha" — hits shared bucket, first entry matches by
        // string compare.
        assert_eq!(
            jit_v2_map_get_str_i64(map, str_vw("alpha")).to_number(),
            Some(111.0)
        );
        // Lookup "beta" — hits its own bucket (or shared if hashes equal);
        // the linear scan finds the correct entry.
        assert_eq!(
            jit_v2_map_get_str_i64(map, str_vw("beta")).to_number(),
            Some(222.0)
        );
    }
}
