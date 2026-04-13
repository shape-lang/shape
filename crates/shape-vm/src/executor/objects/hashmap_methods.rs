//! HashMap method handlers for the PHF method registry.
//!
//! ## Collision Safety
//! All lookups scan the bucket `Vec<usize>` from `HashMapData.index` and check
//! `keys[idx].vw_equals(&key)` for each candidate, so hash collisions never
//! cause silent data loss.

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::type_mismatch_error;
use crate::memory::{record_heap_write, write_barrier_vw};
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HashMapData;
use shape_value::{VMError, ValueWord};
use std::collections::HashMap;

// ─── Helpers ─────────────────────────────────────────────────────────

/// Scan a bucket for the index of a key that equals `needle`.
#[inline]
fn bucket_find(keys: &[ValueWord], bucket: &[usize], needle: &ValueWord) -> Option<usize> {
    bucket
        .iter()
        .copied()
        .find(|&idx| keys[idx].vw_equals(needle))
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 wrappers — raw u64 in/out, zero Vec allocation
// ═══════════════════════════════════════════════════════════════════════════

use crate::executor::objects::raw_helpers;

/// Reconstruct an owned ValueWord from raw bits by cloning (incrementing refcount).
///
/// Used by mutating methods (set, delete, merge) that need `&mut self`.
#[inline]
fn own_vw(raw: u64) -> ValueWord {
    unsafe { ValueWord::clone_from_bits(raw) }
}

/// HashMap.get(key) -> value | none
pub fn v2_get(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(data) = raw_helpers::extract_hashmap_data(args[0]) {
        // Shape-guarded fast path for string keys
        if let Some(ks) = raw_helpers::extract_str(args[1]) {
            if let Some(val) = data.shape_get(ks) {
                return Ok(val.clone().into_raw_bits());
            }
        }
        // Fallback: hash-based lookup with a borrowed key for hashing/equality
        let key = own_vw(args[1]);
        let hash = key.vw_hash();
        let result = if let Some(bucket) = data.index.get(&hash) {
            bucket_find(&data.keys, bucket, &key)
                .map(|idx| data.values[idx].clone())
                .unwrap_or_else(ValueWord::none)
        } else {
            ValueWord::none()
        };
        Ok(result.into_raw_bits())
    } else {
        Err(type_mismatch_error("get", "HashMap"))
    }
}

/// HashMap.set(key, value) -> HashMap (returns new or mutated HashMap)
pub fn v2_set(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let key = own_vw(args[1]);
    let value = own_vw(args[2]);

    // Try mutable fast-path via an owned clone.
    let mut receiver = own_vw(args[0]);
    if let Some(data) = receiver.as_hashmap_mut() {
        let hash = key.vw_hash();
        if let Some(bucket) = data.index.get(&hash) {
            if let Some(idx) = bucket_find(&data.keys, bucket, &key) {
                record_heap_write();
                write_barrier_vw(&data.keys[idx], &key);
                write_barrier_vw(&data.values[idx], &value);
                data.keys[idx] = key;
                data.values[idx] = value;
                return Ok(receiver.into_raw_bits());
            }
        }
        // New key: transition shape if string key, else drop to dictionary mode
        if let Some(shape_id) = data.shape_id {
            if let Some(ks) = key.as_str() {
                let prop_hash = shape_value::hash_property_name(ks);
                data.shape_id = shape_value::shape_transition(shape_id, prop_hash);
            } else {
                data.shape_id = None;
            }
        }
        let idx = data.keys.len();
        data.keys.push(key);
        data.values.push(value);
        data.index.entry(hash).or_default().push(idx);
        return Ok(receiver.into_raw_bits());
    }

    // Slow path: clone everything.
    if let Some((old_keys, old_values, old_index)) = raw_helpers::extract_hashmap(args[0]) {
        let mut keys = old_keys.clone();
        let mut values = old_values.clone();
        let mut index = old_index.clone();

        let hash = key.vw_hash();
        if let Some(bucket) = index.get(&hash) {
            if let Some(idx) = bucket_find(&keys, bucket, &key) {
                keys[idx] = key;
                values[idx] = value;
                return Ok(ValueWord::from_hashmap(keys, values, index).into_raw_bits());
            }
        }
        let idx = keys.len();
        keys.push(key);
        values.push(value);
        index.entry(hash).or_default().push(idx);
        Ok(ValueWord::from_hashmap(keys, values, index).into_raw_bits())
    } else {
        Err(type_mismatch_error("set", "HashMap"))
    }
}

/// HashMap.has(key) -> bool
pub fn v2_has(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some((keys, _, index)) = raw_helpers::extract_hashmap(args[0]) {
        let key = own_vw(args[1]);
        let hash = key.vw_hash();
        let found = index
            .get(&hash)
            .map(|bucket| bucket_find(keys, bucket, &key).is_some())
            .unwrap_or(false);
        Ok(ValueWord::from_bool(found).raw_bits())
    } else {
        Err(type_mismatch_error("has", "HashMap"))
    }
}

/// HashMap.delete(key) -> HashMap (returns new or mutated HashMap)
pub fn v2_delete(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let key = own_vw(args[1]);
    let hash = key.vw_hash();

    // Mutable fast-path via owned clone
    let mut receiver = own_vw(args[0]);
    if let Some(data) = receiver.as_hashmap_mut() {
        if let Some(bucket) = data.index.get(&hash).cloned() {
            if let Some(idx) = bucket_find(&data.keys, &bucket, &key) {
                data.shape_id = None;
                let last = data.keys.len() - 1;
                if idx != last {
                    record_heap_write();
                    write_barrier_vw(&data.keys[idx], &data.keys[last]);
                    write_barrier_vw(&data.values[idx], &data.values[last]);
                    data.keys.swap(idx, last);
                    data.values.swap(idx, last);
                    let swapped_hash = data.keys[idx].vw_hash();
                    if let Some(b) = data.index.get_mut(&swapped_hash) {
                        if let Some(pos) = b.iter().position(|&x| x == last) {
                            b[pos] = idx;
                        }
                    }
                }
                data.keys.pop();
                data.values.pop();
                if let Some(b) = data.index.get_mut(&hash) {
                    b.retain(|&x| x != last);
                    if b.is_empty() {
                        data.index.remove(&hash);
                    }
                }
                return Ok(receiver.into_raw_bits());
            }
        }
        return Ok(receiver.into_raw_bits());
    }

    // Slow path: clone without the deleted key
    if let Some((old_keys, old_values, old_index)) = raw_helpers::extract_hashmap(args[0]) {
        if let Some(bucket) = old_index.get(&hash) {
            if let Some(idx) = bucket_find(old_keys, bucket, &key) {
                let keys: Vec<ValueWord> = old_keys
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != idx)
                    .map(|(_, v)| v.clone())
                    .collect();
                let values: Vec<ValueWord> = old_values
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != idx)
                    .map(|(_, v)| v.clone())
                    .collect();
                let index = HashMapData::rebuild_index(&keys);
                return Ok(ValueWord::from_hashmap(keys, values, index).into_raw_bits());
            }
        }
        Ok(own_vw(args[0]).into_raw_bits())
    } else {
        Err(type_mismatch_error("delete", "HashMap"))
    }
}

/// HashMap.keys() -> Array
pub fn v2_keys(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some((keys, _, _)) = raw_helpers::extract_hashmap(args[0]) {
        let arr = shape_value::vmarray_from_value_words(keys.clone());
        Ok(ValueWord::from_array(arr).into_raw_bits())
    } else {
        Err(type_mismatch_error("keys", "HashMap"))
    }
}

/// HashMap.values() -> Array
pub fn v2_values(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some((_, values, _)) = raw_helpers::extract_hashmap(args[0]) {
        let arr = shape_value::vmarray_from_value_words(values.clone());
        Ok(ValueWord::from_array(arr).into_raw_bits())
    } else {
        Err(type_mismatch_error("values", "HashMap"))
    }
}

/// HashMap.entries() -> Array of [key, value] pairs
pub fn v2_entries(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some((keys, values, _)) = raw_helpers::extract_hashmap(args[0]) {
        let entries: Vec<ValueWord> = keys
            .iter()
            .zip(values.iter())
            .map(|(k, v)| {
                let pair = shape_value::vmarray_from_value_words(vec![k.clone(), v.clone()]);
                ValueWord::from_array(pair)
            })
            .collect();
        let arr = shape_value::vmarray_from_value_words(entries);
        Ok(ValueWord::from_array(arr).into_raw_bits())
    } else {
        Err(type_mismatch_error("entries", "HashMap"))
    }
}

/// HashMap.len() -> int
pub fn v2_len(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some((keys, _, _)) = raw_helpers::extract_hashmap(args[0]) {
        Ok(ValueWord::from_i64(keys.len() as i64).raw_bits())
    } else {
        Err(type_mismatch_error("len", "HashMap"))
    }
}

/// HashMap.isEmpty() -> bool
pub fn v2_is_empty(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some((keys, _, _)) = raw_helpers::extract_hashmap(args[0]) {
        Ok(ValueWord::from_bool(keys.is_empty()).raw_bits())
    } else {
        Err(type_mismatch_error("isEmpty", "HashMap"))
    }
}

/// HashMap.merge(other) -> HashMap (other wins on conflict)
pub fn v2_merge(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (base_keys, base_values, _) = raw_helpers::extract_hashmap(args[0])
        .ok_or_else(|| type_mismatch_error("merge", "HashMap"))?;
    let (other_keys, other_values, _) = raw_helpers::extract_hashmap(args[1])
        .ok_or_else(|| VMError::RuntimeError("merge argument must be a HashMap".to_string()))?;

    let mut keys = base_keys.clone();
    let mut values = base_values.clone();
    let mut index = HashMapData::rebuild_index(&keys);

    for (ok, ov) in other_keys.iter().zip(other_values.iter()) {
        let hash = ok.vw_hash();
        if let Some(bucket) = index.get(&hash) {
            if let Some(idx) = bucket_find(&keys, bucket, ok) {
                keys[idx] = ok.clone();
                values[idx] = ov.clone();
                continue;
            }
        }
        let idx = keys.len();
        keys.push(ok.clone());
        values.push(ov.clone());
        index.entry(hash).or_default().push(idx);
    }

    Ok(ValueWord::from_hashmap(keys, values, index).into_raw_bits())
}

/// HashMap.getOrDefault(key, default) -> value
pub fn v2_get_or_default(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some((keys, values, index)) = raw_helpers::extract_hashmap(args[0]) {
        let key = own_vw(args[1]);
        let hash = key.vw_hash();
        let result = if let Some(bucket) = index.get(&hash) {
            bucket_find(keys, bucket, &key)
                .map(|idx| values[idx].clone())
                .unwrap_or_else(|| own_vw(args[2]))
        } else {
            own_vw(args[2])
        };
        Ok(result.into_raw_bits())
    } else {
        Err(type_mismatch_error("getOrDefault", "HashMap"))
    }
}

/// HashMap.toArray() -> Array of [key, value] pairs (alias for entries)
pub fn v2_to_array(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    v2_entries(vm, args, ctx)
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 closure-based HashMap methods
// ═══════════════════════════════════════════════════════════════════════════

/// HashMap.forEach(fn(key, value)) -> unit [v2]
pub fn v2_for_each(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some((keys, values, _)) = raw_helpers::extract_hashmap(args[0]) {
        let keys = keys.clone();
        let values = values.clone();
        for (k, v) in keys.iter().zip(values.iter()) {
            let result_bits = vm.call_value_immediate_raw(args[1], &[k.raw_bits(), v.raw_bits()], ctx.as_deref_mut())?;
            drop(ValueWord::from_raw_bits(result_bits));
        }
        Ok(ValueWord::unit().raw_bits())
    } else {
        Err(type_mismatch_error("forEach", "HashMap"))
    }
}

/// HashMap.filter(fn(key, value) -> bool) -> HashMap [v2]
pub fn v2_filter(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some((old_keys, old_values, _)) = raw_helpers::extract_hashmap(args[0]) {
        let old_keys = old_keys.clone();
        let old_values = old_values.clone();
        let mut keys = Vec::new();
        let mut values = Vec::new();

        for (k, v) in old_keys.iter().zip(old_values.iter()) {
            let result_bits =
                vm.call_value_immediate_raw(args[1], &[k.raw_bits(), v.raw_bits()], ctx.as_deref_mut())?;
            if raw_helpers::is_truthy_raw(result_bits) {
                keys.push(k.clone());
                values.push(v.clone());
            }
            drop(ValueWord::from_raw_bits(result_bits));
        }

        Ok(ValueWord::from_hashmap_pairs(keys, values).into_raw_bits())
    } else {
        Err(type_mismatch_error("filter", "HashMap"))
    }
}

/// HashMap.map(fn(key, value) -> new_value) -> HashMap [v2]
pub fn v2_map(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some((old_keys, old_values, old_index)) = raw_helpers::extract_hashmap(args[0]) {
        let old_keys = old_keys.clone();
        let old_values = old_values.clone();
        let old_index = old_index.clone();
        let mut new_values = Vec::with_capacity(old_values.len());

        for (k, v) in old_keys.iter().zip(old_values.iter()) {
            let result_bits =
                vm.call_value_immediate_raw(args[1], &[k.raw_bits(), v.raw_bits()], ctx.as_deref_mut())?;
            new_values.push(ValueWord::from_raw_bits(result_bits));
        }

        Ok(ValueWord::from_hashmap(old_keys.clone(), new_values, old_index).into_raw_bits())
    } else {
        Err(type_mismatch_error("map", "HashMap"))
    }
}

/// HashMap.reduce(fn(acc, key, value) -> acc, initial) -> value [v2]
pub fn v2_reduce(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some((keys, values, _)) = raw_helpers::extract_hashmap(args[0]) {
        let keys = keys.clone();
        let values = values.clone();
        let mut acc_bits = raw_helpers::clone_raw_bits(args[2]);
        for (k, v) in keys.iter().zip(values.iter()) {
            let result_bits = vm.call_value_immediate_raw(
                args[1],
                &[acc_bits, k.raw_bits(), v.raw_bits()],
                ctx.as_deref_mut(),
            )?;
            drop(ValueWord::from_raw_bits(acc_bits));
            acc_bits = result_bits;
        }
        Ok(acc_bits)
    } else {
        Err(type_mismatch_error("reduce", "HashMap"))
    }
}

/// HashMap.groupBy(fn(key, value) -> group_key) -> HashMap<group_key, HashMap> [v2]
pub fn v2_group_by(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some((keys, values, _)) = raw_helpers::extract_hashmap(args[0]) {
        let keys = keys.clone();
        let values = values.clone();

        // group_key -> (keys, values) for each sub-hashmap
        let mut groups: Vec<(ValueWord, Vec<ValueWord>, Vec<ValueWord>)> = Vec::new();
        let mut group_index: HashMap<u64, Vec<usize>> = HashMap::new();

        for (k, v) in keys.iter().zip(values.iter()) {
            let result_bits =
                vm.call_value_immediate_raw(args[1], &[k.raw_bits(), v.raw_bits()], ctx.as_deref_mut())?;
            let group_key = ValueWord::from_raw_bits(result_bits);
            let gk_hash = group_key.vw_hash();

            let found = if let Some(bucket) = group_index.get(&gk_hash) {
                bucket
                    .iter()
                    .copied()
                    .find(|&gi| groups[gi].0.vw_equals(&group_key))
            } else {
                None
            };

            if let Some(gi) = found {
                groups[gi].1.push(k.clone());
                groups[gi].2.push(v.clone());
            } else {
                let gi = groups.len();
                groups.push((group_key, vec![k.clone()], vec![v.clone()]));
                group_index.entry(gk_hash).or_default().push(gi);
            }
        }

        // Build outer HashMap: group_key -> inner HashMap
        let outer_keys: Vec<ValueWord> = groups.iter().map(|(gk, _, _)| gk.clone()).collect();
        let outer_values: Vec<ValueWord> = groups
            .into_iter()
            .map(|(_, ks, vs)| ValueWord::from_hashmap_pairs(ks, vs))
            .collect();

        Ok(ValueWord::from_hashmap_pairs(outer_keys, outer_values).into_raw_bits())
    } else {
        Err(type_mismatch_error("groupBy", "HashMap"))
    }
}

/// HashMap.iter() -> Iterator [v2]
pub fn v2_iter(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    use shape_value::heap_value::IteratorState;
    let receiver = unsafe { ValueWord::clone_from_bits(args[0]) };
    let result = ValueWord::from_iterator(Box::new(IteratorState {
        source: receiver,
        position: 0,
        transforms: vec![],
        done: false,
    }));
    Ok(result.into_raw_bits())
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::ValueWord;
    use shape_value::heap_value::HashMapData;
    use std::sync::Arc;

    fn nb_str(s: &str) -> ValueWord {
        ValueWord::from_string(Arc::new(s.to_string()))
    }

    /// Build a test HashMap: {"a": 1, "b": 2, "c": 3}
    fn test_hashmap() -> ValueWord {
        let keys = vec![nb_str("a"), nb_str("b"), nb_str("c")];
        let values = vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
            ValueWord::from_i64(3),
        ];
        ValueWord::from_hashmap_pairs(keys, values)
    }

    // ===== Collision safety unit tests =====

    #[test]
    fn test_find_key_with_equality_check() {
        let keys = vec![nb_str("alpha"), nb_str("beta")];
        let values = vec![ValueWord::from_i64(1), ValueWord::from_i64(2)];
        let index = HashMapData::rebuild_index(&keys);

        let data = HashMapData {
            keys,
            values,
            index,
            shape_id: None,
        };
        assert_eq!(data.find_key(&nb_str("alpha")), Some(0));
        assert_eq!(data.find_key(&nb_str("beta")), Some(1));
        assert_eq!(data.find_key(&nb_str("gamma")), None);
    }

    #[test]
    fn test_collision_bucket_chaining() {
        let k1 = nb_str("key1");
        let k2 = nb_str("key2");
        let keys = vec![k1.clone(), k2.clone()];
        let values = vec![ValueWord::from_i64(10), ValueWord::from_i64(20)];

        // Force both into same bucket to simulate collision
        let h1 = k1.vw_hash();
        let h2 = k2.vw_hash();
        let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
        index.insert(h1, vec![0, 1]);
        if h2 != h1 {
            index.insert(h2, vec![1]);
        }

        let data = HashMapData {
            keys,
            values,
            index,
            shape_id: None,
        };
        assert_eq!(data.find_key(&k1), Some(0));
        assert_eq!(data.find_key(&k2), Some(1));
    }

    #[test]
    fn test_set_does_not_lose_data_on_hash_collision() {
        let hm = test_hashmap();
        let (keys, _, _) = hm.as_hashmap().unwrap();
        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn test_has_checks_equality_not_just_hash() {
        let keys = vec![nb_str("x")];
        let values = vec![ValueWord::from_i64(1)];
        let hm = ValueWord::from_hashmap_pairs(keys, values);
        let (keys_ref, _, index) = hm.as_hashmap().unwrap();

        let hash = nb_str("x").vw_hash();
        let bucket = index.get(&hash).unwrap();
        assert!(bucket_find(keys_ref, bucket, &nb_str("x")).is_some());
        assert!(bucket_find(keys_ref, bucket, &nb_str("y")).is_none());
    }

    #[test]
    fn test_delete_preserves_other_entries() {
        let new_keys = vec![nb_str("a"), nb_str("c")];
        let new_values = vec![ValueWord::from_i64(1), ValueWord::from_i64(3)];
        let new_index = HashMapData::rebuild_index(&new_keys);
        let data = HashMapData {
            keys: new_keys,
            values: new_values,
            index: new_index,
            shape_id: None,
        };
        assert_eq!(data.find_key(&nb_str("a")), Some(0));
        assert_eq!(data.find_key(&nb_str("c")), Some(1));
        assert_eq!(data.find_key(&nb_str("b")), None);
    }

    #[test]
    fn test_rebuild_index_produces_correct_buckets() {
        let keys = vec![nb_str("a"), nb_str("b"), nb_str("c")];
        let index = HashMapData::rebuild_index(&keys);

        for (i, k) in keys.iter().enumerate() {
            let hash = k.vw_hash();
            let bucket = index.get(&hash).unwrap();
            assert!(bucket.contains(&i));
        }
    }

    #[test]
    fn test_from_hashmap_pairs_round_trip() {
        let keys = vec![nb_str("x"), nb_str("y")];
        let values = vec![ValueWord::from_i64(10), ValueWord::from_i64(20)];
        let hm = ValueWord::from_hashmap_pairs(keys, values);

        let (k, v, idx) = hm.as_hashmap().unwrap();
        assert_eq!(k.len(), 2);
        assert_eq!(v.len(), 2);
        assert!(idx.get(&nb_str("x").vw_hash()).is_some());
        assert!(idx.get(&nb_str("y").vw_hash()).is_some());
    }

    #[test]
    fn test_integer_key_hash_consistency() {
        let k1 = ValueWord::from_i64(42);
        let k2 = ValueWord::from_i64(42);
        assert_eq!(k1.vw_hash(), k2.vw_hash());
        assert!(k1.vw_equals(&k2));
    }

    #[test]
    fn test_empty_hashmap_find_key() {
        let hm = ValueWord::empty_hashmap();
        let (keys, _, index) = hm.as_hashmap().unwrap();
        assert!(keys.is_empty());
        assert!(index.is_empty());
    }
}
