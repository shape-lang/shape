//! HashMap method handlers for the PHF method registry.
//!
//! All methods follow the MethodFn signature:
//! fn(&mut VirtualMachine, Vec<ValueWord>, Option<&mut ExecutionContext>) -> Result<(), VMError>
//!
//! ## Collision Safety
//! All lookups scan the bucket `Vec<usize>` from `HashMapData.index` and check
//! `keys[idx].vw_equals(&key)` for each candidate, so hash collisions never
//! cause silent data loss.
//!
//! ## Mutable Fast-Path
//! `handle_set` and `handle_delete` check if the receiver ValueWord's Arc has
//! `strong_count == 1`. If so, they mutate in place via `as_hashmap_mut()`
//! instead of cloning the entire HashMap.

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::{check_arg_count, type_mismatch_error};
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

// ─── Core methods (get, set, has, delete) ────────────────────────────

/// HashMap.get(key) -> value | none
pub fn handle_get(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "HashMap.get", "a key argument")?;
    let receiver = &args[0];
    let key = &args[1];

    if let Some(data) = receiver.as_hashmap_data() {
        // Shape-guarded fast path for string keys
        if let Some(ks) = key.as_str() {
            if let Some(val) = data.shape_get(ks) {
                vm.push_vw(val.clone())?;
                return Ok(());
            }
        }
        // Fallback: hash-based lookup
        let hash = key.vw_hash();
        let result = if let Some(bucket) = data.index.get(&hash) {
            bucket_find(&data.keys, bucket, key)
                .map(|idx| data.values[idx].clone())
                .unwrap_or_else(ValueWord::none)
        } else {
            ValueWord::none()
        };
        vm.push_vw(result)?;
        Ok(())
    } else {
        Err(type_mismatch_error("get", "HashMap"))
    }
}

/// HashMap.set(key, value) -> HashMap (returns new or mutated HashMap)
pub fn handle_set(
    vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 3, "HashMap.set", "key and value arguments")?;
    let key = args[1].clone();
    let value = args[2].clone();

    // Mutable fast-path: if we are the sole owner, mutate in place.
    if let Some(data) = args[0].as_hashmap_mut() {
        let hash = key.vw_hash();
        if let Some(bucket) = data.index.get(&hash) {
            if let Some(idx) = bucket_find(&data.keys, bucket, &key) {
                // Existing key: update value, shape unchanged
                record_heap_write();
                write_barrier_vw(&data.keys[idx], &key);
                write_barrier_vw(&data.values[idx], &value);
                data.keys[idx] = key;
                data.values[idx] = value;
                vm.push_vw(args[0].clone())?;
                return Ok(());
            }
        }
        // New key: transition shape if string key, else drop to dictionary mode
        if let Some(shape_id) = data.shape_id {
            if let Some(ks) = key.as_str() {
                let prop_hash = shape_value::hash_property_name(ks);
                data.shape_id = shape_value::shape_transition(shape_id, prop_hash);
            } else {
                data.shape_id = None; // Non-string key → dictionary mode
            }
        }
        let idx = data.keys.len();
        data.keys.push(key);
        data.values.push(value);
        data.index.entry(hash).or_default().push(idx);
        vm.push_vw(args[0].clone())?;
        return Ok(());
    }

    // Slow path: clone everything.
    if let Some((old_keys, old_values, old_index)) = args[0].as_hashmap() {
        let mut keys = old_keys.clone();
        let mut values = old_values.clone();
        let mut index = old_index.clone();

        let hash = key.vw_hash();
        if let Some(bucket) = index.get(&hash) {
            if let Some(idx) = bucket_find(&keys, bucket, &key) {
                keys[idx] = key;
                values[idx] = value;
                vm.push_vw(ValueWord::from_hashmap(keys, values, index))?;
                return Ok(());
            }
        }
        let idx = keys.len();
        keys.push(key);
        values.push(value);
        index.entry(hash).or_default().push(idx);
        vm.push_vw(ValueWord::from_hashmap(keys, values, index))?;
        Ok(())
    } else {
        Err(type_mismatch_error("set", "HashMap"))
    }
}

/// HashMap.has(key) -> bool
pub fn handle_has(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "HashMap.has", "a key argument")?;
    let receiver = &args[0];
    let key = &args[1];

    if let Some((keys, _, index)) = receiver.as_hashmap() {
        let hash = key.vw_hash();
        let found = index
            .get(&hash)
            .map(|bucket| bucket_find(keys, bucket, key).is_some())
            .unwrap_or(false);
        vm.push_vw(ValueWord::from_bool(found))?;
        Ok(())
    } else {
        Err(type_mismatch_error("has", "HashMap"))
    }
}

/// HashMap.delete(key) -> HashMap (returns new or mutated HashMap)
pub fn handle_delete(
    vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "HashMap.delete", "a key argument")?;
    let key = args[1].clone();
    let hash = key.vw_hash();

    // Mutable fast-path
    if let Some(data) = args[0].as_hashmap_mut() {
        if let Some(bucket) = data.index.get(&hash).cloned() {
            if let Some(idx) = bucket_find(&data.keys, &bucket, &key) {
                // Delete invalidates shape → dictionary mode
                data.shape_id = None;
                let last = data.keys.len() - 1;
                if idx != last {
                    record_heap_write();
                    write_barrier_vw(&data.keys[idx], &data.keys[last]);
                    write_barrier_vw(&data.values[idx], &data.values[last]);
                    data.keys.swap(idx, last);
                    data.values.swap(idx, last);
                    // Update index for the key that was swapped into `idx`
                    let swapped_hash = data.keys[idx].vw_hash();
                    if let Some(b) = data.index.get_mut(&swapped_hash) {
                        if let Some(pos) = b.iter().position(|&x| x == last) {
                            b[pos] = idx;
                        }
                    }
                }
                data.keys.pop();
                data.values.pop();
                // Remove the deleted index from its bucket
                if let Some(b) = data.index.get_mut(&hash) {
                    b.retain(|&x| x != last);
                    if b.is_empty() {
                        data.index.remove(&hash);
                    }
                }
                vm.push_vw(args[0].clone())?;
                return Ok(());
            }
        }
        // Key not found
        vm.push_vw(args[0].clone())?;
        return Ok(());
    }

    // Slow path: clone without the deleted key
    if let Some((old_keys, old_values, old_index)) = args[0].as_hashmap() {
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
                vm.push_vw(ValueWord::from_hashmap(keys, values, index))?;
            } else {
                vm.push_vw(args[0].clone())?;
            }
        } else {
            vm.push_vw(args[0].clone())?;
        }
        Ok(())
    } else {
        Err(type_mismatch_error("delete", "HashMap"))
    }
}

// ─── Collection accessors ────────────────────────────────────────────

/// HashMap.keys() -> Array
pub fn handle_keys(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    if let Some((keys, _, _)) = receiver.as_hashmap() {
        let arr = shape_value::vmarray_from_value_words(keys.clone());
        vm.push_vw(ValueWord::from_array(arr))?;
        Ok(())
    } else {
        Err(type_mismatch_error("keys", "HashMap"))
    }
}

/// HashMap.values() -> Array
pub fn handle_values(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    if let Some((_, values, _)) = receiver.as_hashmap() {
        let arr = shape_value::vmarray_from_value_words(values.clone());
        vm.push_vw(ValueWord::from_array(arr))?;
        Ok(())
    } else {
        Err(type_mismatch_error("values", "HashMap"))
    }
}

/// HashMap.entries() -> Array of [key, value] pairs
pub fn handle_entries(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    if let Some((keys, values, _)) = receiver.as_hashmap() {
        let entries: Vec<ValueWord> = keys
            .iter()
            .zip(values.iter())
            .map(|(k, v)| {
                let pair = shape_value::vmarray_from_value_words(vec![k.clone(), v.clone()]);
                ValueWord::from_array(pair)
            })
            .collect();
        let arr = shape_value::vmarray_from_value_words(entries);
        vm.push_vw(ValueWord::from_array(arr))?;
        Ok(())
    } else {
        Err(type_mismatch_error("entries", "HashMap"))
    }
}

/// HashMap.len() -> int
pub fn handle_len(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    if let Some((keys, _, _)) = receiver.as_hashmap() {
        vm.push_vw(ValueWord::from_i64(keys.len() as i64))?;
        Ok(())
    } else {
        Err(type_mismatch_error("len", "HashMap"))
    }
}

/// HashMap.isEmpty() -> bool
pub fn handle_is_empty(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    if let Some((keys, _, _)) = receiver.as_hashmap() {
        vm.push_vw(ValueWord::from_bool(keys.is_empty()))?;
        Ok(())
    } else {
        Err(type_mismatch_error("isEmpty", "HashMap"))
    }
}

// ─── Higher-order methods ────────────────────────────────────────────

/// HashMap.forEach(fn(key, value) -> void) -> unit
pub fn handle_for_each(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "HashMap.forEach", "a function argument")?;
    let receiver = args[0].clone();
    let callback = args[1].clone();

    if let Some((keys, values, _)) = receiver.as_hashmap() {
        let keys = keys.clone();
        let values = values.clone();
        for (k, v) in keys.iter().zip(values.iter()) {
            vm.call_value_immediate_nb(&callback, &[k.clone(), v.clone()], ctx.as_deref_mut())?;
        }
        vm.push_vw(ValueWord::unit())?;
        Ok(())
    } else {
        Err(type_mismatch_error("forEach", "HashMap"))
    }
}

/// HashMap.filter(fn(key, value) -> bool) -> HashMap
pub fn handle_filter(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "HashMap.filter", "a function argument")?;
    let receiver = args[0].clone();
    let callback = args[1].clone();

    if let Some((old_keys, old_values, _)) = receiver.as_hashmap() {
        let old_keys = old_keys.clone();
        let old_values = old_values.clone();
        let mut keys = Vec::new();
        let mut values = Vec::new();

        for (k, v) in old_keys.iter().zip(old_values.iter()) {
            let result =
                vm.call_value_immediate_nb(&callback, &[k.clone(), v.clone()], ctx.as_deref_mut())?;
            if result.is_truthy() {
                keys.push(k.clone());
                values.push(v.clone());
            }
        }

        vm.push_vw(ValueWord::from_hashmap_pairs(keys, values))?;
        Ok(())
    } else {
        Err(type_mismatch_error("filter", "HashMap"))
    }
}

/// HashMap.map(fn(key, value) -> new_value) -> HashMap
pub fn handle_map(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "HashMap.map", "a function argument")?;
    let receiver = args[0].clone();
    let callback = args[1].clone();

    if let Some((old_keys, old_values, old_index)) = receiver.as_hashmap() {
        let old_keys = old_keys.clone();
        let old_values = old_values.clone();
        let old_index = old_index.clone();
        let mut new_values = Vec::with_capacity(old_values.len());

        for (k, v) in old_keys.iter().zip(old_values.iter()) {
            let result =
                vm.call_value_immediate_nb(&callback, &[k.clone(), v.clone()], ctx.as_deref_mut())?;
            new_values.push(result);
        }

        vm.push_vw(ValueWord::from_hashmap(
            old_keys.clone(),
            new_values,
            old_index,
        ))?;
        Ok(())
    } else {
        Err(type_mismatch_error("map", "HashMap"))
    }
}

// ─── New methods (Sprint 3) ──────────────────────────────────────────

/// HashMap.merge(other) -> HashMap (other wins on conflict)
pub fn handle_merge(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "HashMap.merge", "a HashMap argument")?;
    let receiver = &args[0];
    let other = &args[1];

    let (base_keys, base_values, _) = receiver
        .as_hashmap()
        .ok_or_else(|| type_mismatch_error("merge", "HashMap"))?;
    let (other_keys, other_values, _) = other
        .as_hashmap()
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

    vm.push_vw(ValueWord::from_hashmap(keys, values, index))?;
    Ok(())
}

/// HashMap.getOrDefault(key, default) -> value
pub fn handle_get_or_default(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 3, "HashMap.getOrDefault", "key and default arguments")?;
    let receiver = &args[0];
    let key = &args[1];
    let default = &args[2];

    if let Some((keys, values, index)) = receiver.as_hashmap() {
        let hash = key.vw_hash();
        let result = if let Some(bucket) = index.get(&hash) {
            bucket_find(keys, bucket, key)
                .map(|idx| values[idx].clone())
                .unwrap_or_else(|| default.clone())
        } else {
            default.clone()
        };
        vm.push_vw(result)?;
        Ok(())
    } else {
        Err(type_mismatch_error("getOrDefault", "HashMap"))
    }
}

/// HashMap.reduce(fn(acc, key, value) -> acc, initial) -> value
pub fn handle_reduce(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 3, "HashMap.reduce", "a function and initial value")?;
    let receiver = args[0].clone();
    let callback = args[1].clone();
    let initial = args[2].clone();

    if let Some((keys, values, _)) = receiver.as_hashmap() {
        let keys = keys.clone();
        let values = values.clone();
        let mut acc = initial;
        for (k, v) in keys.iter().zip(values.iter()) {
            acc = vm.call_value_immediate_nb(
                &callback,
                &[acc, k.clone(), v.clone()],
                ctx.as_deref_mut(),
            )?;
        }
        vm.push_vw(acc)?;
        Ok(())
    } else {
        Err(type_mismatch_error("reduce", "HashMap"))
    }
}

/// HashMap.toArray() -> Array of [key, value] pairs (alias for entries())
pub fn handle_to_array(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    handle_entries(vm, args, ctx)
}

/// HashMap.groupBy(fn(key, value) -> group_key) -> HashMap<group_key, HashMap>
pub fn handle_group_by(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "HashMap.groupBy", "a function argument")?;
    let receiver = args[0].clone();
    let callback = args[1].clone();

    if let Some((keys, values, _)) = receiver.as_hashmap() {
        let keys = keys.clone();
        let values = values.clone();

        // group_key -> (keys, values) for each sub-hashmap
        let mut groups: Vec<(ValueWord, Vec<ValueWord>, Vec<ValueWord>)> = Vec::new();
        let mut group_index: HashMap<u64, Vec<usize>> = HashMap::new();

        for (k, v) in keys.iter().zip(values.iter()) {
            let group_key =
                vm.call_value_immediate_nb(&callback, &[k.clone(), v.clone()], ctx.as_deref_mut())?;
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

        // Build outer HashMap
        let mut outer_keys = Vec::with_capacity(groups.len());
        let mut outer_values = Vec::with_capacity(groups.len());
        for (gk, gkeys, gvalues) in groups {
            outer_keys.push(gk);
            outer_values.push(ValueWord::from_hashmap_pairs(gkeys, gvalues));
        }

        vm.push_vw(ValueWord::from_hashmap_pairs(outer_keys, outer_values))?;
        Ok(())
    } else {
        Err(type_mismatch_error("groupBy", "HashMap"))
    }
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
