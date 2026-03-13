//! Content-addressed state diffing for distributed Shape.
//!
//! Provides `diff(old, new)` and `patch(base, delta)` operations that
//! compare values using content-hash trees. Only changed subtrees are
//! included in the delta, enabling efficient state synchronization.

use crate::hashing::HashDigest;
use crate::type_schema::TypeSchemaRegistry;
use sha2::{Digest, Sha256};
use shape_value::NanTag;
use shape_value::ValueWord;
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Delta representation
// ---------------------------------------------------------------------------

/// A delta between two values, keyed by content path.
///
/// Paths use dot-separated notation:
/// - `"field_name"` for top-level fields of a TypedObject
/// - `"field_name.nested"` for nested fields
/// - `"[0]"`, `"[1]"` for array indices
/// - `"frames.[0].locals.[2]"` for deeply nested paths
#[derive(Debug, Clone)]
pub struct Delta {
    /// Fields/paths that changed, mapped to their new values.
    pub changed: HashMap<String, ValueWord>,
    /// Paths that were removed (present in old, absent in new).
    pub removed: Vec<String>,
}

impl Delta {
    /// Create an empty delta (no changes).
    pub fn empty() -> Self {
        Self {
            changed: HashMap::new(),
            removed: Vec::new(),
        }
    }

    /// True if this delta represents no change.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty()
    }

    /// Number of changes (additions + modifications + removals).
    pub fn change_count(&self) -> usize {
        self.changed.len() + self.removed.len()
    }

    /// Apply this delta to a base value, producing the updated value.
    ///
    /// This is a convenience wrapper around [`patch_value`] that validates
    /// delta paths before applying. Invalid paths (empty segments, leading
    /// or trailing dots) are silently skipped.
    ///
    /// # Path validation
    ///
    /// Each path in `changed` and `removed` is checked for basic structural
    /// validity:
    /// - Must not be empty (except the root sentinel `"."`).
    /// - Must not contain empty segments (e.g. `"a..b"`).
    /// - Must not start or end with `"."` (except the root sentinel).
    ///
    /// Paths that fail validation are excluded from the applied delta and
    /// collected into the returned `Vec` of rejected path strings.
    pub fn patch(
        &self,
        base: &ValueWord,
        schemas: &TypeSchemaRegistry,
    ) -> (ValueWord, Vec<String>) {
        let mut rejected = Vec::new();
        let validated = self.validated_delta(&mut rejected);
        let result = patch_value(base, &validated, schemas);
        (result, rejected)
    }

    /// Build a new `Delta` containing only paths that pass validation,
    /// collecting rejected paths into `rejected`.
    fn validated_delta(&self, rejected: &mut Vec<String>) -> Delta {
        let mut valid = Delta::empty();

        for (path, value) in &self.changed {
            if is_valid_delta_path(path) {
                valid.changed.insert(path.clone(), value.clone());
            } else {
                rejected.push(path.clone());
            }
        }

        for path in &self.removed {
            if is_valid_delta_path(path) {
                valid.removed.push(path.clone());
            } else {
                rejected.push(path.clone());
            }
        }

        valid
    }
}

/// Check whether a delta path is structurally valid.
///
/// The root sentinel `"."` is always valid. All other paths must be
/// non-empty, must not contain empty segments (consecutive dots), and
/// must not start or end with a dot.
fn is_valid_delta_path(path: &str) -> bool {
    // Root sentinel is always valid
    if path == "." {
        return true;
    }

    if path.is_empty() {
        return false;
    }

    // Array index paths like "[0]" are valid
    if path.starts_with('[') {
        return true;
    }

    // Must not start or end with a dot
    if path.starts_with('.') || path.ends_with('.') {
        return false;
    }

    // Must not contain empty segments (consecutive dots)
    if path.contains("..") {
        return false;
    }

    true
}

// ---------------------------------------------------------------------------
// Value hashing
// ---------------------------------------------------------------------------

/// Compute a content hash for a ValueWord value.
///
/// Provides structural hashing that is deterministic across runs.
/// For TypedObjects, fields are hashed in slot order. For arrays, each
/// element is hashed. Primitives are hashed by their binary representation.
pub fn content_hash_value(value: &ValueWord, schemas: &TypeSchemaRegistry) -> HashDigest {
    let mut hasher = Sha256::new();
    hash_value_into(&mut hasher, value, schemas);
    let result = hasher.finalize();
    let hex_str = result.iter().fold(String::with_capacity(64), |mut acc, b| {
        use std::fmt::Write;
        let _ = write!(acc, "{:02x}", b);
        acc
    });
    HashDigest::from_hex(&hex_str)
}

fn hash_value_into(hasher: &mut Sha256, value: &ValueWord, schemas: &TypeSchemaRegistry) {
    match value.tag() {
        NanTag::F64 => {
            hasher.update(b"f64:");
            if let Some(f) = value.as_f64() {
                hasher.update(f.to_le_bytes());
            }
        }
        NanTag::I48 => {
            hasher.update(b"i48:");
            if let Some(i) = value.as_i64() {
                hasher.update(i.to_le_bytes());
            }
        }
        NanTag::Bool => {
            hasher.update(b"bool:");
            if let Some(b) = value.as_bool() {
                hasher.update(if b { &[1u8] } else { &[0u8] });
            }
        }
        NanTag::None => {
            hasher.update(b"none");
        }
        NanTag::Unit => {
            hasher.update(b"unit");
        }
        NanTag::Function => {
            hasher.update(b"fn:");
            hasher.update(value.raw_bits().to_le_bytes());
        }
        NanTag::ModuleFunction => {
            hasher.update(b"modfn:");
            hasher.update(value.raw_bits().to_le_bytes());
        }
        NanTag::Ref => {
            hasher.update(b"ref:");
            hasher.update(value.raw_bits().to_le_bytes());
        }
        NanTag::Heap => {
            // Heap values: differentiate by content
            if let Some(s) = value.as_str() {
                hasher.update(b"str:");
                hasher.update((s.len() as u64).to_le_bytes());
                hasher.update(s.as_bytes());
            } else if let Some(view) = value.as_any_array() {
                hasher.update(b"arr:");
                hasher.update((view.len() as u64).to_le_bytes());
                let arr = view.to_generic();
                for elem in arr.iter() {
                    hash_value_into(hasher, elem, schemas);
                }
            } else if let Some((schema_id, slots, heap_mask)) = value.as_typed_object() {
                hasher.update(b"obj:");
                hasher.update(schema_id.to_le_bytes());
                for (i, slot) in slots.iter().enumerate() {
                    let is_heap = (heap_mask >> i) & 1 == 1;
                    if is_heap {
                        let nb = slot.as_heap_nb();
                        hash_value_into(hasher, &nb, schemas);
                    } else {
                        hasher.update(b"slot:");
                        hasher.update(slot.raw().to_le_bytes());
                    }
                }
            } else {
                // Other heap types (BigInt, Decimal, Closure, etc.)
                hasher.update(b"heap:");
                hasher.update(value.raw_bits().to_le_bytes());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Diffing
// ---------------------------------------------------------------------------

/// Compute the delta between two values.
///
/// For TypedObjects of the same schema, produces per-field diffs.
/// For arrays of the same length, produces per-element diffs.
/// For all other cases, treats the entire value as changed if different.
pub fn diff_values(old: &ValueWord, new: &ValueWord, schemas: &TypeSchemaRegistry) -> Delta {
    let mut delta = Delta::empty();
    diff_recursive(old, new, "", schemas, &mut delta);
    delta
}

fn make_path(prefix: &str, suffix: &str) -> String {
    if prefix.is_empty() {
        suffix.to_string()
    } else {
        format!("{}.{}", prefix, suffix)
    }
}

fn root_path(prefix: &str) -> String {
    if prefix.is_empty() {
        ".".to_string()
    } else {
        prefix.to_string()
    }
}

fn diff_recursive(
    old: &ValueWord,
    new: &ValueWord,
    prefix: &str,
    schemas: &TypeSchemaRegistry,
    delta: &mut Delta,
) {
    // Fast path: identical raw bits means identical value
    if old.raw_bits() == new.raw_bits() {
        return;
    }

    // If tags differ, the whole subtree changed
    if old.tag() != new.tag() {
        delta.changed.insert(root_path(prefix), new.clone());
        return;
    }

    match old.tag() {
        NanTag::Heap => {
            // Try typed object diff
            if let (Some((old_sid, old_slots, old_hm)), Some((new_sid, new_slots, new_hm))) =
                (old.as_typed_object(), new.as_typed_object())
            {
                if old_sid == new_sid {
                    let schema = schemas.get_by_id(old_sid as u32);
                    let min_len = old_slots.len().min(new_slots.len());

                    for i in 0..min_len {
                        let field_name = schema
                            .and_then(|s| s.fields.get(i).map(|f| f.name.as_str()))
                            .unwrap_or("?");
                        let field_path = make_path(prefix, field_name);

                        let old_is_heap = (old_hm >> i) & 1 == 1;
                        let new_is_heap = (new_hm >> i) & 1 == 1;

                        if old_is_heap && new_is_heap {
                            let old_nb = old_slots[i].as_heap_nb();
                            let new_nb = new_slots[i].as_heap_nb();
                            diff_recursive(&old_nb, &new_nb, &field_path, schemas, delta);
                        } else if old_slots[i].raw() != new_slots[i].raw()
                            || old_is_heap != new_is_heap
                        {
                            // Slot raw bits differ or heap-ness changed
                            if new_is_heap {
                                delta.changed.insert(field_path, new_slots[i].as_heap_nb());
                            } else {
                                delta.changed.insert(field_path, unsafe {
                                    ValueWord::clone_from_bits(new_slots[i].raw())
                                });
                            }
                        }
                    }

                    // Extra new slots
                    for i in old_slots.len()..new_slots.len() {
                        let field_name = schema
                            .and_then(|s| s.fields.get(i).map(|f| f.name.as_str()))
                            .unwrap_or("?");
                        let field_path = make_path(prefix, field_name);
                        let is_heap = (new_hm >> i) & 1 == 1;
                        if is_heap {
                            delta.changed.insert(field_path, new_slots[i].as_heap_nb());
                        } else {
                            delta.changed.insert(field_path, unsafe {
                                ValueWord::clone_from_bits(new_slots[i].raw())
                            });
                        }
                    }

                    // Removed slots
                    for i in new_slots.len()..old_slots.len() {
                        let field_name = schema
                            .and_then(|s| s.fields.get(i).map(|f| f.name.as_str()))
                            .unwrap_or("?");
                        delta.removed.push(make_path(prefix, field_name));
                    }
                    return;
                }
                // Different schemas: whole value changed
                delta.changed.insert(root_path(prefix), new.clone());
                return;
            }

            // Try array diff
            if let (Some(old_view), Some(new_view)) = (old.as_any_array(), new.as_any_array()) {
                let old_arr = old_view.to_generic();
                let new_arr = new_view.to_generic();
                let min_len = old_arr.len().min(new_arr.len());

                for i in 0..min_len {
                    let idx_path = if prefix.is_empty() {
                        format!("[{}]", i)
                    } else {
                        format!("{}.[{}]", prefix, i)
                    };
                    diff_recursive(&old_arr[i], &new_arr[i], &idx_path, schemas, delta);
                }

                for i in min_len..new_arr.len() {
                    let idx_path = if prefix.is_empty() {
                        format!("[{}]", i)
                    } else {
                        format!("{}.[{}]", prefix, i)
                    };
                    delta.changed.insert(idx_path, new_arr[i].clone());
                }

                for i in min_len..old_arr.len() {
                    let idx_path = if prefix.is_empty() {
                        format!("[{}]", i)
                    } else {
                        format!("{}.[{}]", prefix, i)
                    };
                    delta.removed.push(idx_path);
                }
                return;
            }

            // Try HashMap diff
            if let (Some(old_data), Some(new_data)) =
                (old.as_hashmap_data(), new.as_hashmap_data())
            {
                diff_hashmap(old_data, new_data, prefix, schemas, delta);
                return;
            }

            // Try string diff
            if let (Some(old_s), Some(new_s)) = (old.as_str(), new.as_str()) {
                if old_s != new_s {
                    delta.changed.insert(root_path(prefix), new.clone());
                }
                return;
            }

            // Different heap subtypes: whole value changed
            delta.changed.insert(root_path(prefix), new.clone());
        }

        _ => {
            // Primitive types: already checked raw bits above, so they differ
            delta.changed.insert(root_path(prefix), new.clone());
        }
    }
}

/// Diff two HashMap values by comparing keys and values.
///
/// Detects:
/// - Keys present in `new` but not in `old` (added entries)
/// - Keys present in `old` but not in `new` (removed entries)
/// - Keys present in both but with different values (changed entries)
///
/// For changed entries whose values are themselves compound types (arrays,
/// objects, hashmaps), diffs recursively instead of treating as atomic.
fn diff_hashmap(
    old_data: &shape_value::HashMapData,
    new_data: &shape_value::HashMapData,
    prefix: &str,
    schemas: &TypeSchemaRegistry,
    delta: &mut Delta,
) {
    // Build a lookup from old keys for efficient comparison.
    // For each key in the new map, check if it exists in the old map.
    for (new_idx, new_key) in new_data.keys.iter().enumerate() {
        let key_label = format_map_key(new_key);
        let key_path = make_path(prefix, &key_label);

        match old_data.find_key(new_key) {
            Some(old_idx) => {
                // Key exists in both — diff the values recursively
                diff_recursive(
                    &old_data.values[old_idx],
                    &new_data.values[new_idx],
                    &key_path,
                    schemas,
                    delta,
                );
            }
            None => {
                // Key added in new
                delta
                    .changed
                    .insert(key_path, new_data.values[new_idx].clone());
            }
        }
    }

    // Find keys removed from old (present in old, absent in new)
    for old_key in &old_data.keys {
        if new_data.find_key(old_key).is_none() {
            let key_label = format_map_key(old_key);
            let key_path = make_path(prefix, &key_label);
            delta.removed.push(key_path);
        }
    }
}

/// Format a HashMap key as a path component for delta paths.
///
/// String keys use their value directly (e.g. `"name"`).
/// Integer keys use bracket notation (e.g. `{42}`).
/// Other types use a debug-style representation.
fn format_map_key(key: &ValueWord) -> String {
    if let Some(s) = key.as_str() {
        s.to_string()
    } else if let Some(i) = key.as_i64() {
        format!("{{{}}}", i)
    } else if let Some(f) = key.as_f64() {
        format!("{{{}}}", f)
    } else if let Some(b) = key.as_bool() {
        format!("{{{}}}", b)
    } else {
        format!("{{0x{:x}}}", key.raw_bits())
    }
}

// ---------------------------------------------------------------------------
// Patching
// ---------------------------------------------------------------------------

/// Apply a delta to a base value, producing the updated value.
///
/// For TypedObjects, patches individual fields by path.
/// For arrays, patches individual elements by index.
/// For root-level changes (path "."), replaces the entire value.
pub fn patch_value(base: &ValueWord, delta: &Delta, schemas: &TypeSchemaRegistry) -> ValueWord {
    if delta.is_empty() {
        return base.clone();
    }

    // Root-level replacement
    if let Some(root_val) = delta.changed.get(".") {
        return root_val.clone();
    }

    // Try to patch TypedObject fields
    if let Some((schema_id, slots, heap_mask)) = base.as_typed_object() {
        let schema = schemas.get_by_id(schema_id as u32);
        if let Some(schema) = schema {
            // Partition changed entries into direct and nested
            let mut direct_changes: HashMap<String, ValueWord> = HashMap::new();
            let mut nested_changes: HashMap<String, Delta> = HashMap::new();

            for (path, value) in &delta.changed {
                if let Some(dot_pos) = path.find('.') {
                    let top = &path[..dot_pos];
                    let rest = &path[dot_pos + 1..];
                    nested_changes
                        .entry(top.to_string())
                        .or_insert_with(Delta::empty)
                        .changed
                        .insert(rest.to_string(), value.clone());
                } else {
                    direct_changes.insert(path.clone(), value.clone());
                }
            }

            // Similarly partition removed entries into direct and nested.
            // Note: direct removals for TypedObject fields are not currently
            // applied (fields can't be removed from a fixed schema), but we
            // still partition so nested removals are forwarded recursively.
            let mut _direct_removals: Vec<String> = Vec::new();
            let mut nested_removals: HashMap<String, Delta> = HashMap::new();

            for path in &delta.removed {
                if let Some(dot_pos) = path.find('.') {
                    let top = &path[..dot_pos];
                    let rest = &path[dot_pos + 1..];
                    nested_removals
                        .entry(top.to_string())
                        .or_insert_with(Delta::empty)
                        .removed
                        .push(rest.to_string());
                } else {
                    _direct_removals.push(path.clone());
                }
            }

            // Merge nested removals into nested_changes map
            for (top, mut removal_delta) in nested_removals {
                let entry = nested_changes.entry(top).or_insert_with(Delta::empty);
                entry.removed.append(&mut removal_delta.removed);
            }

            // Clone all slots carefully
            let mut new_slots: Vec<shape_value::ValueSlot> = Vec::with_capacity(slots.len());
            for (i, slot) in slots.iter().enumerate() {
                let is_heap = (heap_mask >> i) & 1 == 1;
                if is_heap {
                    new_slots.push(unsafe { slot.clone_heap() });
                } else {
                    new_slots.push(shape_value::ValueSlot::from_raw(slot.raw()));
                }
            }
            let mut new_heap_mask = heap_mask;

            // Apply direct field changes (paths with no '.' separator)
            for (path, new_val) in &direct_changes {
                if let Some(field_idx_u16) = schema.field_index(path) {
                    let field_idx = field_idx_u16 as usize;
                    if field_idx < new_slots.len() {
                        // Drop old heap slot if needed
                        if (new_heap_mask >> field_idx) & 1 == 1 {
                            unsafe {
                                new_slots[field_idx].drop_heap();
                            }
                        }

                        if new_val.is_heap() {
                            if let Some(hv) = new_val.as_heap_ref() {
                                new_slots[field_idx] =
                                    shape_value::ValueSlot::from_heap(hv.clone());
                                new_heap_mask |= 1u64 << field_idx;
                            }
                        } else if let Some(f) = new_val.as_f64() {
                            new_slots[field_idx] = shape_value::ValueSlot::from_number(f);
                            new_heap_mask &= !(1u64 << field_idx);
                        } else if let Some(i) = new_val.as_i64() {
                            new_slots[field_idx] = shape_value::ValueSlot::from_int(i);
                            new_heap_mask &= !(1u64 << field_idx);
                        } else if let Some(b) = new_val.as_bool() {
                            new_slots[field_idx] = shape_value::ValueSlot::from_bool(b);
                            new_heap_mask &= !(1u64 << field_idx);
                        }
                    }
                }
            }

            // Apply nested field changes (dotted paths like "inner.field")
            for (top_field, sub_delta) in &nested_changes {
                if let Some(field_idx_u16) = schema.field_index(top_field) {
                    let field_idx = field_idx_u16 as usize;
                    if field_idx < new_slots.len() {
                        // Extract the current value from the slot
                        let is_heap = (new_heap_mask >> field_idx) & 1 == 1;
                        if is_heap {
                            let current_val = new_slots[field_idx].as_heap_nb();
                            // Recursively patch the nested value
                            let patched = patch_value(&current_val, sub_delta, schemas);

                            // Drop the old heap slot
                            unsafe {
                                new_slots[field_idx].drop_heap();
                            }

                            // Write back the patched value
                            if patched.is_heap() {
                                if let Some(hv) = patched.as_heap_ref() {
                                    new_slots[field_idx] =
                                        shape_value::ValueSlot::from_heap(hv.clone());
                                    new_heap_mask |= 1u64 << field_idx;
                                }
                            } else if let Some(f) = patched.as_f64() {
                                new_slots[field_idx] = shape_value::ValueSlot::from_number(f);
                                new_heap_mask &= !(1u64 << field_idx);
                            } else if let Some(i) = patched.as_i64() {
                                new_slots[field_idx] = shape_value::ValueSlot::from_int(i);
                                new_heap_mask &= !(1u64 << field_idx);
                            } else if let Some(b) = patched.as_bool() {
                                new_slots[field_idx] = shape_value::ValueSlot::from_bool(b);
                                new_heap_mask &= !(1u64 << field_idx);
                            }
                        }
                    }
                }
            }

            use shape_value::HeapValue;
            return ValueWord::from_heap_value(HeapValue::TypedObject {
                schema_id,
                slots: new_slots.into_boxed_slice(),
                heap_mask: new_heap_mask,
            });
        }
    }

    // Try to patch Array elements
    if let Some(view) = base.as_any_array() {
        let arr = view.to_generic();
        let mut new_arr: Vec<ValueWord> = arr.to_vec();

        // Process removals first (high to low to preserve indices)
        let mut removal_indices: Vec<usize> = delta
            .removed
            .iter()
            .filter_map(|path| parse_array_index(path))
            .collect();
        removal_indices.sort_unstable();
        removal_indices.reverse();
        for idx in removal_indices {
            if idx < new_arr.len() {
                new_arr.remove(idx);
            }
        }

        // Process changes
        for (path, new_val) in &delta.changed {
            if let Some(idx) = parse_array_index(path) {
                if idx < new_arr.len() {
                    new_arr[idx] = new_val.clone();
                } else {
                    while new_arr.len() < idx {
                        new_arr.push(ValueWord::none());
                    }
                    new_arr.push(new_val.clone());
                }
            }
        }

        return ValueWord::from_array(Arc::new(new_arr));
    }

    // Try to patch HashMap entries
    if let Some(data) = base.as_hashmap_data() {
        let mut new_keys = data.keys.clone();
        let mut new_values = data.values.clone();

        // Process removals
        for path in &delta.removed {
            // Find the key in the map and remove it
            let remove_idx = new_keys
                .iter()
                .position(|k| format_map_key(k) == *path);
            if let Some(idx) = remove_idx {
                new_keys.remove(idx);
                new_values.remove(idx);
            }
        }

        // Process changes (add or update)
        for (path, new_val) in &delta.changed {
            // Check if this path has nested sub-paths (contains '.')
            // For simplicity, direct key changes are applied here.
            let existing_idx = new_keys
                .iter()
                .position(|k| format_map_key(k) == *path);
            if let Some(idx) = existing_idx {
                new_values[idx] = new_val.clone();
            } else {
                // New key — use a string key matching the path label
                new_keys.push(ValueWord::from_string(Arc::new(path.clone())));
                new_values.push(new_val.clone());
            }
        }

        return ValueWord::from_hashmap_pairs(new_keys, new_values);
    }

    // Cannot patch — return base unchanged
    base.clone()
}

/// Parse an array index from a path like "[3]" or "prefix.[3]".
fn parse_array_index(path: &str) -> Option<usize> {
    let part = path.rsplit('.').next().unwrap_or(path);
    if part.starts_with('[') && part.ends_with(']') {
        part[1..part.len() - 1].parse().ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_delta() {
        let delta = Delta::empty();
        assert!(delta.is_empty());
        assert_eq!(delta.change_count(), 0);
    }

    #[test]
    fn test_diff_identical_primitives() {
        let schemas = TypeSchemaRegistry::new();
        let a = ValueWord::from_f64(42.0);
        let b = ValueWord::from_f64(42.0);
        let delta = diff_values(&a, &b, &schemas);
        assert!(delta.is_empty());
    }

    #[test]
    fn test_diff_different_primitives() {
        let schemas = TypeSchemaRegistry::new();
        let a = ValueWord::from_f64(42.0);
        let b = ValueWord::from_f64(99.0);
        let delta = diff_values(&a, &b, &schemas);
        assert!(!delta.is_empty());
        assert_eq!(delta.change_count(), 1);
        assert!(delta.changed.contains_key("."));
    }

    #[test]
    fn test_diff_arrays_same() {
        let schemas = TypeSchemaRegistry::new();
        let a = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
        ]));
        let b = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
        ]));
        let delta = diff_values(&a, &b, &schemas);
        // Different Arc pointers so raw bits differ, but elements match
        assert!(delta.is_empty());
    }

    #[test]
    fn test_diff_arrays_element_changed() {
        let schemas = TypeSchemaRegistry::new();
        let a = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
        ]));
        let b = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(99.0),
        ]));
        let delta = diff_values(&a, &b, &schemas);
        assert_eq!(delta.change_count(), 1);
        assert!(delta.changed.contains_key("[1]"));
    }

    #[test]
    fn test_diff_arrays_element_added() {
        let schemas = TypeSchemaRegistry::new();
        let a = ValueWord::from_array(Arc::new(vec![ValueWord::from_f64(1.0)]));
        let b = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
        ]));
        let delta = diff_values(&a, &b, &schemas);
        assert_eq!(delta.changed.len(), 1);
        assert!(delta.changed.contains_key("[1]"));
    }

    #[test]
    fn test_diff_arrays_element_removed() {
        let schemas = TypeSchemaRegistry::new();
        let a = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
        ]));
        let b = ValueWord::from_array(Arc::new(vec![ValueWord::from_f64(1.0)]));
        let delta = diff_values(&a, &b, &schemas);
        assert_eq!(delta.removed.len(), 1);
        assert!(delta.removed.contains(&"[1]".to_string()));
    }

    #[test]
    fn test_patch_root_replacement() {
        let schemas = TypeSchemaRegistry::new();
        let base = ValueWord::from_f64(42.0);
        let mut delta = Delta::empty();
        delta
            .changed
            .insert(".".to_string(), ValueWord::from_f64(99.0));

        let result = patch_value(&base, &delta, &schemas);
        assert_eq!(result.as_f64(), Some(99.0));
    }

    #[test]
    fn test_patch_array_element() {
        let schemas = TypeSchemaRegistry::new();
        let base = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
        ]));
        let mut delta = Delta::empty();
        delta
            .changed
            .insert("[1]".to_string(), ValueWord::from_f64(99.0));

        let result = patch_value(&base, &delta, &schemas);
        let arr = result.as_any_array().unwrap().to_generic();
        assert_eq!(arr[0].as_f64(), Some(1.0));
        assert_eq!(arr[1].as_f64(), Some(99.0));
    }

    #[test]
    fn test_parse_array_index() {
        assert_eq!(parse_array_index("[0]"), Some(0));
        assert_eq!(parse_array_index("[42]"), Some(42));
        assert_eq!(parse_array_index("prefix.[3]"), Some(3));
        assert_eq!(parse_array_index("notindex"), None);
    }

    #[test]
    fn test_content_hash_deterministic() {
        let schemas = TypeSchemaRegistry::new();
        let v1 = ValueWord::from_f64(42.0);
        let v2 = ValueWord::from_f64(42.0);
        assert_eq!(
            content_hash_value(&v1, &schemas),
            content_hash_value(&v2, &schemas)
        );
    }

    #[test]
    fn test_content_hash_different() {
        let schemas = TypeSchemaRegistry::new();
        let v1 = ValueWord::from_f64(42.0);
        let v2 = ValueWord::from_f64(99.0);
        assert_ne!(
            content_hash_value(&v1, &schemas),
            content_hash_value(&v2, &schemas)
        );
    }

    #[test]
    fn test_nested_typed_object_diff_and_patch() {
        use crate::type_schema::TypeSchemaBuilder;
        use shape_value::{HeapValue, ValueSlot};

        let mut schemas = TypeSchemaRegistry::new();

        // Register inner type: Inner { x: f64, y: f64 }
        let inner_id = TypeSchemaBuilder::new("Inner")
            .f64_field("x")
            .f64_field("y")
            .register(&mut schemas);

        // Register outer type: Outer { name: string, inner: Inner, score: f64 }
        let outer_id = TypeSchemaBuilder::new("Outer")
            .string_field("name")
            .object_field("inner", "Inner")
            .f64_field("score")
            .register(&mut schemas);

        // Build inner objects
        let inner_old = ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: inner_id as u64,
            slots: vec![
                ValueSlot::from_number(1.0), // x = 1.0
                ValueSlot::from_number(2.0), // y = 2.0
            ]
            .into_boxed_slice(),
            heap_mask: 0,
        });

        let inner_new = ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: inner_id as u64,
            slots: vec![
                ValueSlot::from_number(1.0),  // x = 1.0 (unchanged)
                ValueSlot::from_number(99.0), // y = 99.0 (changed)
            ]
            .into_boxed_slice(),
            heap_mask: 0,
        });

        // Build outer objects
        let name_val = Arc::new("test".to_string());
        let old_outer = ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: outer_id as u64,
            slots: vec![
                ValueSlot::from_heap(HeapValue::String(name_val.clone())), // name
                ValueSlot::from_heap(inner_old.as_heap_ref().unwrap().clone()), // inner
                ValueSlot::from_number(10.0),                              // score
            ]
            .into_boxed_slice(),
            heap_mask: 0b011, // slots 0 and 1 are heap
        });

        let new_outer = ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: outer_id as u64,
            slots: vec![
                ValueSlot::from_heap(HeapValue::String(name_val.clone())), // name (same)
                ValueSlot::from_heap(inner_new.as_heap_ref().unwrap().clone()), // inner (y changed)
                ValueSlot::from_number(10.0),                              // score (same)
            ]
            .into_boxed_slice(),
            heap_mask: 0b011,
        });

        // Diff should produce a dotted path "inner.y"
        let delta = diff_values(&old_outer, &new_outer, &schemas);
        assert!(!delta.is_empty(), "delta should not be empty");
        assert!(
            delta.changed.contains_key("inner.y"),
            "delta should contain 'inner.y', got keys: {:?}",
            delta.changed.keys().collect::<Vec<_>>()
        );
        assert_eq!(delta.change_count(), 1, "only inner.y should have changed");

        // Patch should correctly apply the nested change
        let patched = patch_value(&old_outer, &delta, &schemas);

        // Verify the patched outer object
        let (patched_sid, patched_slots, patched_hm) = patched
            .as_typed_object()
            .expect("patched should be a TypedObject");
        assert_eq!(patched_sid, outer_id as u64);

        // name should be unchanged
        assert_eq!(patched_hm & 1, 1, "slot 0 should be heap");
        let patched_name = patched_slots[0].as_heap_nb();
        assert_eq!(patched_name.as_str().unwrap(), "test");

        // score should be unchanged
        assert_eq!(
            f64::from_bits(patched_slots[2].raw()),
            10.0,
            "score should be 10.0"
        );

        // inner should have y=99.0 and x=1.0
        assert_eq!((patched_hm >> 1) & 1, 1, "slot 1 should be heap");
        let patched_inner = patched_slots[1].as_heap_nb();
        let (inner_sid, inner_slots, _inner_hm) = patched_inner
            .as_typed_object()
            .expect("inner should be a TypedObject");
        assert_eq!(inner_sid, inner_id as u64);
        assert_eq!(
            f64::from_bits(inner_slots[0].raw()),
            1.0,
            "inner.x should be 1.0"
        );
        assert_eq!(
            f64::from_bits(inner_slots[1].raw()),
            99.0,
            "inner.y should be 99.0"
        );
    }

    #[test]
    fn test_patch_direct_fields_still_work() {
        use crate::type_schema::TypeSchemaBuilder;
        use shape_value::{HeapValue, ValueSlot};

        let mut schemas = TypeSchemaRegistry::new();

        let schema_id = TypeSchemaBuilder::new("Simple")
            .f64_field("a")
            .f64_field("b")
            .register(&mut schemas);

        let base = ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: schema_id as u64,
            slots: vec![ValueSlot::from_number(1.0), ValueSlot::from_number(2.0)]
                .into_boxed_slice(),
            heap_mask: 0,
        });

        // Direct field patch (no dots)
        let mut delta = Delta::empty();
        delta
            .changed
            .insert("b".to_string(), ValueWord::from_f64(42.0));

        let patched = patch_value(&base, &delta, &schemas);
        let (_sid, slots, _hm) = patched.as_typed_object().unwrap();
        assert_eq!(f64::from_bits(slots[0].raw()), 1.0, "a unchanged");
        assert_eq!(f64::from_bits(slots[1].raw()), 42.0, "b patched to 42.0");
    }

    #[test]
    fn test_nested_patch_mixed_direct_and_dotted() {
        use crate::type_schema::TypeSchemaBuilder;
        use shape_value::{HeapValue, ValueSlot};

        let mut schemas = TypeSchemaRegistry::new();

        let inner_id = TypeSchemaBuilder::new("MixedInner")
            .f64_field("val")
            .register(&mut schemas);

        let outer_id = TypeSchemaBuilder::new("MixedOuter")
            .f64_field("score")
            .object_field("nested", "MixedInner")
            .register(&mut schemas);

        let inner_obj = ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: inner_id as u64,
            slots: vec![ValueSlot::from_number(5.0)].into_boxed_slice(),
            heap_mask: 0,
        });

        let base = ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: outer_id as u64,
            slots: vec![
                ValueSlot::from_number(100.0),
                ValueSlot::from_heap(inner_obj.as_heap_ref().unwrap().clone()),
            ]
            .into_boxed_slice(),
            heap_mask: 0b10, // slot 1 is heap
        });

        // Delta with both a direct change and a nested dotted change
        let mut delta = Delta::empty();
        delta
            .changed
            .insert("score".to_string(), ValueWord::from_f64(200.0));
        delta
            .changed
            .insert("nested.val".to_string(), ValueWord::from_f64(77.0));

        let patched = patch_value(&base, &delta, &schemas);
        let (_sid, slots, hm) = patched.as_typed_object().unwrap();

        // Direct field should be patched
        assert_eq!(
            f64::from_bits(slots[0].raw()),
            200.0,
            "score should be 200.0"
        );

        // Nested field should be patched
        assert_eq!((hm >> 1) & 1, 1, "slot 1 should be heap");
        let patched_inner = slots[1].as_heap_nb();
        let (_inner_sid, inner_slots, _) = patched_inner.as_typed_object().unwrap();
        assert_eq!(
            f64::from_bits(inner_slots[0].raw()),
            77.0,
            "nested.val should be 77.0"
        );
    }

    // ---- HashMap diffing tests ----

    #[test]
    fn test_diff_hashmaps_identical() {
        let schemas = TypeSchemaRegistry::new();
        let a = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("x".to_string())),
                ValueWord::from_string(Arc::new("y".to_string())),
            ],
            vec![ValueWord::from_f64(1.0), ValueWord::from_f64(2.0)],
        );
        let b = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("x".to_string())),
                ValueWord::from_string(Arc::new("y".to_string())),
            ],
            vec![ValueWord::from_f64(1.0), ValueWord::from_f64(2.0)],
        );
        let delta = diff_values(&a, &b, &schemas);
        assert!(delta.is_empty(), "identical hashmaps should produce empty delta");
    }

    #[test]
    fn test_diff_hashmaps_value_changed() {
        let schemas = TypeSchemaRegistry::new();
        let a = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("x".to_string())),
                ValueWord::from_string(Arc::new("y".to_string())),
            ],
            vec![ValueWord::from_f64(1.0), ValueWord::from_f64(2.0)],
        );
        let b = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("x".to_string())),
                ValueWord::from_string(Arc::new("y".to_string())),
            ],
            vec![ValueWord::from_f64(1.0), ValueWord::from_f64(99.0)],
        );
        let delta = diff_values(&a, &b, &schemas);
        assert_eq!(delta.change_count(), 1);
        assert!(delta.changed.contains_key("y"));
    }

    #[test]
    fn test_diff_hashmaps_key_added() {
        let schemas = TypeSchemaRegistry::new();
        let a = ValueWord::from_hashmap_pairs(
            vec![ValueWord::from_string(Arc::new("x".to_string()))],
            vec![ValueWord::from_f64(1.0)],
        );
        let b = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("x".to_string())),
                ValueWord::from_string(Arc::new("y".to_string())),
            ],
            vec![ValueWord::from_f64(1.0), ValueWord::from_f64(2.0)],
        );
        let delta = diff_values(&a, &b, &schemas);
        assert_eq!(delta.changed.len(), 1);
        assert!(delta.changed.contains_key("y"));
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn test_diff_hashmaps_key_removed() {
        let schemas = TypeSchemaRegistry::new();
        let a = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("x".to_string())),
                ValueWord::from_string(Arc::new("y".to_string())),
            ],
            vec![ValueWord::from_f64(1.0), ValueWord::from_f64(2.0)],
        );
        let b = ValueWord::from_hashmap_pairs(
            vec![ValueWord::from_string(Arc::new("x".to_string()))],
            vec![ValueWord::from_f64(1.0)],
        );
        let delta = diff_values(&a, &b, &schemas);
        assert!(delta.changed.is_empty());
        assert_eq!(delta.removed.len(), 1);
        assert!(delta.removed.contains(&"y".to_string()));
    }

    #[test]
    fn test_diff_hashmaps_symmetric_difference() {
        // Tests set-like diffing: keys present in one but not the other
        let schemas = TypeSchemaRegistry::new();
        let a = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("a".to_string())),
                ValueWord::from_string(Arc::new("b".to_string())),
                ValueWord::from_string(Arc::new("c".to_string())),
            ],
            vec![
                ValueWord::from_f64(1.0),
                ValueWord::from_f64(2.0),
                ValueWord::from_f64(3.0),
            ],
        );
        let b = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("b".to_string())),
                ValueWord::from_string(Arc::new("c".to_string())),
                ValueWord::from_string(Arc::new("d".to_string())),
            ],
            vec![
                ValueWord::from_f64(2.0),
                ValueWord::from_f64(3.0),
                ValueWord::from_f64(4.0),
            ],
        );
        let delta = diff_values(&a, &b, &schemas);
        // "a" removed, "d" added, "b" and "c" unchanged
        assert_eq!(delta.removed.len(), 1);
        assert!(delta.removed.contains(&"a".to_string()));
        assert_eq!(delta.changed.len(), 1);
        assert!(delta.changed.contains_key("d"));
    }

    #[test]
    fn test_diff_hashmap_with_integer_keys() {
        let schemas = TypeSchemaRegistry::new();
        let a = ValueWord::from_hashmap_pairs(
            vec![ValueWord::from_i64(1), ValueWord::from_i64(2)],
            vec![
                ValueWord::from_string(Arc::new("one".to_string())),
                ValueWord::from_string(Arc::new("two".to_string())),
            ],
        );
        let b = ValueWord::from_hashmap_pairs(
            vec![ValueWord::from_i64(1), ValueWord::from_i64(2)],
            vec![
                ValueWord::from_string(Arc::new("one".to_string())),
                ValueWord::from_string(Arc::new("TWO".to_string())),
            ],
        );
        let delta = diff_values(&a, &b, &schemas);
        assert_eq!(delta.change_count(), 1);
        // Integer key 2 should be formatted as {2}
        assert!(delta.changed.contains_key("{2}"));
    }

    #[test]
    fn test_patch_hashmap_add_entry() {
        let schemas = TypeSchemaRegistry::new();
        let base = ValueWord::from_hashmap_pairs(
            vec![ValueWord::from_string(Arc::new("x".to_string()))],
            vec![ValueWord::from_f64(1.0)],
        );
        let mut delta = Delta::empty();
        delta
            .changed
            .insert("y".to_string(), ValueWord::from_f64(2.0));

        let patched = patch_value(&base, &delta, &schemas);
        let data = patched.as_hashmap_data().expect("should be hashmap");
        assert_eq!(data.keys.len(), 2);
    }

    #[test]
    fn test_patch_hashmap_remove_entry() {
        let schemas = TypeSchemaRegistry::new();
        let base = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("x".to_string())),
                ValueWord::from_string(Arc::new("y".to_string())),
            ],
            vec![ValueWord::from_f64(1.0), ValueWord::from_f64(2.0)],
        );
        let mut delta = Delta::empty();
        delta.removed.push("y".to_string());

        let patched = patch_value(&base, &delta, &schemas);
        let data = patched.as_hashmap_data().expect("should be hashmap");
        assert_eq!(data.keys.len(), 1);
        assert!(data.find_key(&ValueWord::from_string(Arc::new("x".to_string()))).is_some());
    }

    // ---- Nested array diffing tests ----

    #[test]
    fn test_diff_nested_arrays_recursive() {
        let schemas = TypeSchemaRegistry::new();
        // Array of arrays: [[1, 2], [3, 4]]
        let inner1_old = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
        ]));
        let inner2 = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(3.0),
            ValueWord::from_f64(4.0),
        ]));
        let a = ValueWord::from_array(Arc::new(vec![inner1_old, inner2.clone()]));

        // Change inner array [0][1] from 2.0 to 99.0
        let inner1_new = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(99.0),
        ]));
        let b = ValueWord::from_array(Arc::new(vec![inner1_new, inner2]));

        let delta = diff_values(&a, &b, &schemas);
        // Should recursively diff and produce [0].[1] as changed
        assert_eq!(delta.change_count(), 1, "only one element changed");
        assert!(
            delta.changed.contains_key("[0].[1]"),
            "should have path [0].[1], got keys: {:?}",
            delta.changed.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_diff_nested_array_with_object_elements() {
        use crate::type_schema::TypeSchemaBuilder;
        use shape_value::{HeapValue, ValueSlot};

        let mut schemas = TypeSchemaRegistry::new();
        let point_id = TypeSchemaBuilder::new("Point")
            .f64_field("x")
            .f64_field("y")
            .register(&mut schemas);

        let mk_point = |x: f64, y: f64| {
            ValueWord::from_heap_value(HeapValue::TypedObject {
                schema_id: point_id as u64,
                slots: vec![ValueSlot::from_number(x), ValueSlot::from_number(y)]
                    .into_boxed_slice(),
                heap_mask: 0,
            })
        };

        let a = ValueWord::from_array(Arc::new(vec![mk_point(1.0, 2.0), mk_point(3.0, 4.0)]));
        let b = ValueWord::from_array(Arc::new(vec![mk_point(1.0, 2.0), mk_point(3.0, 99.0)]));

        let delta = diff_values(&a, &b, &schemas);
        // Should recursively diff: [1].y changed
        assert_eq!(delta.change_count(), 1);
        assert!(
            delta.changed.contains_key("[1].y"),
            "should have path [1].y, got keys: {:?}",
            delta.changed.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_diff_hashmap_nested_value_recursive() {
        // HashMap with array values — changes within the array should be
        // detected recursively.
        let schemas = TypeSchemaRegistry::new();

        let old_arr = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
        ]));
        let new_arr = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(99.0),
        ]));

        let a = ValueWord::from_hashmap_pairs(
            vec![ValueWord::from_string(Arc::new("data".to_string()))],
            vec![old_arr],
        );
        let b = ValueWord::from_hashmap_pairs(
            vec![ValueWord::from_string(Arc::new("data".to_string()))],
            vec![new_arr],
        );
        let delta = diff_values(&a, &b, &schemas);
        // Should recursively diff: data.[1] changed
        assert_eq!(delta.change_count(), 1);
        assert!(
            delta.changed.contains_key("data.[1]"),
            "should have path data.[1], got keys: {:?}",
            delta.changed.keys().collect::<Vec<_>>()
        );
    }

    // ---- Path validation tests ----

    #[test]
    fn test_is_valid_delta_path_root() {
        assert!(super::is_valid_delta_path("."));
    }

    #[test]
    fn test_is_valid_delta_path_simple_field() {
        assert!(super::is_valid_delta_path("name"));
        assert!(super::is_valid_delta_path("field_name"));
    }

    #[test]
    fn test_is_valid_delta_path_dotted() {
        assert!(super::is_valid_delta_path("a.b.c"));
        assert!(super::is_valid_delta_path("inner.field"));
    }

    #[test]
    fn test_is_valid_delta_path_array_index() {
        assert!(super::is_valid_delta_path("[0]"));
        assert!(super::is_valid_delta_path("[42]"));
    }

    #[test]
    fn test_is_valid_delta_path_rejects_empty() {
        assert!(!super::is_valid_delta_path(""));
    }

    #[test]
    fn test_is_valid_delta_path_rejects_leading_dot() {
        assert!(!super::is_valid_delta_path(".field"));
    }

    #[test]
    fn test_is_valid_delta_path_rejects_trailing_dot() {
        assert!(!super::is_valid_delta_path("field."));
    }

    #[test]
    fn test_is_valid_delta_path_rejects_empty_segment() {
        assert!(!super::is_valid_delta_path("a..b"));
    }

    // ---- Delta::patch() tests ----

    #[test]
    fn test_delta_patch_valid_paths() {
        let schemas = TypeSchemaRegistry::new();
        let base = ValueWord::from_f64(42.0);
        let mut delta = Delta::empty();
        delta
            .changed
            .insert(".".to_string(), ValueWord::from_f64(99.0));

        let (result, rejected) = delta.patch(&base, &schemas);
        assert!(rejected.is_empty());
        assert_eq!(result.as_f64(), Some(99.0));
    }

    #[test]
    fn test_delta_patch_rejects_invalid_paths() {
        let schemas = TypeSchemaRegistry::new();
        let base = ValueWord::from_f64(42.0);
        let mut delta = Delta::empty();
        // Valid path
        delta
            .changed
            .insert(".".to_string(), ValueWord::from_f64(99.0));
        // Invalid paths
        delta
            .changed
            .insert("".to_string(), ValueWord::from_f64(1.0));
        delta
            .changed
            .insert("a..b".to_string(), ValueWord::from_f64(2.0));
        delta.removed.push(".trailing.".to_string());

        let (result, rejected) = delta.patch(&base, &schemas);
        assert_eq!(rejected.len(), 3);
        assert!(rejected.contains(&"".to_string()));
        assert!(rejected.contains(&"a..b".to_string()));
        assert!(rejected.contains(&".trailing.".to_string()));
        // The valid root replacement should still apply
        assert_eq!(result.as_f64(), Some(99.0));
    }
}
