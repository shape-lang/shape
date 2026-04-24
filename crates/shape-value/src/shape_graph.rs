//! Shape graph for HashMap hidden classes.
//!
//! A "shape" describes the ordered property layout of a HashMap, enabling
//! O(1) index-based property access instead of hash-based lookup. This is
//! the same concept as V8's "hidden classes" or "maps".
//!
//! Shapes form a transition tree: adding a property transitions to a child
//! shape. Multiple HashMaps with the same property insertion order share
//! the same shape, enabling inline caching.

use std::collections::HashMap;
use std::fmt;

/// Unique identifier for a shape in the transition graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShapeId(pub u32);

impl fmt::Display for ShapeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Shape({})", self.0)
    }
}

/// A shape describes the ordered property layout of a HashMap.
///
/// Properties are identified by u32 "string IDs" (hashed property names).
/// The index of a property in the `properties` vec is its slot index for
/// direct access.
#[derive(Debug, Clone)]
pub struct Shape {
    pub id: ShapeId,
    /// Ordered property names (as string IDs/hashes).
    pub properties: Vec<u32>,
    /// Parent shape in the transition chain (None for root).
    pub parent: Option<ShapeId>,
    /// Number of properties (== properties.len(), cached for speed).
    pub property_count: u16,
}

/// Manages shape creation and transitions.
///
/// Thread-safety: This is NOT thread-safe. For multi-threaded use,
/// wrap in a Mutex or use thread-local instances.
pub struct ShapeTransitionTable {
    /// Transition edges: (parent_shape, property_name_id) -> child_shape
    transitions: HashMap<(ShapeId, u32), ShapeId>,
    /// All shapes, indexed by ShapeId
    shapes: Vec<Shape>,
    /// Next shape ID to assign
    next_id: u32,
}

impl ShapeTransitionTable {
    /// Create a new transition table with a root shape (id=0, no properties).
    pub fn new() -> Self {
        let root = Shape {
            id: ShapeId(0),
            properties: Vec::new(),
            parent: None,
            property_count: 0,
        };
        Self {
            transitions: HashMap::new(),
            shapes: vec![root],
            next_id: 1,
        }
    }

    /// Returns the root shape ID (always ShapeId(0)).
    #[inline]
    pub fn root() -> ShapeId {
        ShapeId(0)
    }

    /// Get a shape by its ID. Panics if the ID is invalid.
    #[inline]
    pub fn get_shape(&self, id: ShapeId) -> &Shape {
        &self.shapes[id.0 as usize]
    }

    /// Get a shape by its ID, returning `None` if the ID is not present in
    /// this table.
    ///
    /// Use this when a `ShapeId` may have originated from a *different*
    /// transition table (for example, a `HashMapData::shape_id` computed
    /// under the process-default ambient handle but observed inside a
    /// per-VM scope). In that case callers should degrade to the
    /// hash-based slow path rather than panic on an out-of-range index.
    #[inline]
    pub fn try_get_shape(&self, id: ShapeId) -> Option<&Shape> {
        self.shapes.get(id.0 as usize)
    }

    /// Transition from `from` shape by adding `property_name`.
    ///
    /// Returns the existing child shape if the transition already exists,
    /// otherwise creates a new shape with the property appended.
    pub fn transition(&mut self, from: ShapeId, property_name: u32) -> ShapeId {
        if let Some(&existing) = self.transitions.get(&(from, property_name)) {
            return existing;
        }

        let parent_shape = &self.shapes[from.0 as usize];
        let mut properties = parent_shape.properties.clone();
        properties.push(property_name);

        let new_id = ShapeId(self.next_id);
        self.next_id += 1;

        let new_shape = Shape {
            id: new_id,
            properties,
            parent: Some(from),
            property_count: self.shapes[from.0 as usize].property_count + 1,
        };

        self.shapes.push(new_shape);
        self.transitions.insert((from, property_name), new_id);
        new_id
    }

    /// Build a shape by transitioning from root through all keys in order.
    pub fn shape_for_keys(&mut self, keys: &[u32]) -> ShapeId {
        let mut current = Self::root();
        for &key in keys {
            current = self.transition(current, key);
        }
        current
    }

    /// Find the slot index of `property_name` in the given shape.
    ///
    /// Returns `None` if the property is not part of this shape, or if
    /// `shape_id` does not belong to this table (e.g. a stale ID carried
    /// over from a different ambient handle — see `try_get_shape`).
    /// O(n) scan of the shape's properties list.
    pub fn property_index(&self, shape_id: ShapeId, property_name: u32) -> Option<usize> {
        let shape = self.shapes.get(shape_id.0 as usize)?;
        shape.properties.iter().position(|&p| p == property_name)
    }

    /// Total number of shapes in the table.
    #[inline]
    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }
}

// ── Ambient shape table (task-local / thread-local) ────────────────────────
//
// Pre-B5 this module owned a process-global
// `LazyLock<Mutex<ShapeTransitionTable>>` plus a sibling
// `LazyLock<Mutex<Vec<(ShapeId, ShapeId)>>>` transition log. Those
// statics were the single source of truth for every HashMap shape
// transition and for the JIT tier-manager's shape-guard invalidation
// drain.
//
// B5 replaces those statics with a per-VM `ShapeTableHandle` installed
// via `shape_graph_current` (see module docs there). These free
// functions now look up the ambient handle on entry — if one is
// installed (i.e. we're inside a VM execution scope, either via
// `SyncShapeTableScope` or `with_async_shape_table_scope`) they
// operate against that per-VM table; otherwise they return the same
// "degrade gracefully" values (`None` / empty) that the old impl
// returned on lock poisoning, so callers outside any VM scope (e.g.
// unit tests that build a `HashMapData` directly) continue to work.

/// Drain all pending shape transition events.
///
/// Returns the events accumulated since the last drain. Each event is
/// `(parent_shape_id, new_child_shape_id)`.
///
/// Called by `TierManager::check_shape_invalidations()` during `poll_completions()`.
/// When no shape-table scope is active returns an empty `Vec` (no transitions
/// are tracked in that state).
pub fn drain_shape_transitions() -> Vec<(ShapeId, ShapeId)> {
    let Some(handle) = crate::shape_graph_current::try_current_shape_table() else {
        return Vec::new();
    };
    handle
        .transition_log()
        .lock()
        .map(|mut log| std::mem::take(&mut *log))
        .unwrap_or_default()
}

/// Compute a ShapeId for a HashMap with the given key hashes (in insertion order).
///
/// Consults the ambient shape table (see module docs). Returns `None` if no
/// scope is active, if the lock is poisoned, or if there are more than 64
/// properties (dictionary mode threshold).
pub fn shape_for_hashmap_keys(key_hashes: &[u32]) -> Option<ShapeId> {
    if key_hashes.len() > 64 {
        return None; // Dictionary mode: too many properties
    }
    let handle = crate::shape_graph_current::try_current_shape_table()?;
    let mut table = handle.table().lock().ok()?;
    Some(table.shape_for_keys(key_hashes))
}

/// Look up the slot index of a property in a shape.
///
/// Consults the ambient shape table. Returns `None` if no scope is active.
pub fn shape_property_index(shape_id: ShapeId, property_hash: u32) -> Option<usize> {
    let handle = crate::shape_graph_current::try_current_shape_table()?;
    let table = handle.table().lock().ok()?;
    table.property_index(shape_id, property_hash)
}

/// Transition a shape by adding a new property.
///
/// Consults the ambient shape table. Returns `None` if no scope is active,
/// if the dictionary-mode threshold (>64 properties) would be exceeded, or
/// if `from` is not present in the active table (e.g. a stale shape_id
/// carried over from a different ambient handle — a `HashMapData` built
/// under the process-default handle and later mutated inside a per-VM
/// scope). When a transition is recorded, it is also appended to the
/// ambient table's transition log for JIT shape-guard invalidation.
pub fn shape_transition(from: ShapeId, property_hash: u32) -> Option<ShapeId> {
    let handle = crate::shape_graph_current::try_current_shape_table()?;
    let mut table = handle.table().lock().ok()?;
    let shape = table.try_get_shape(from)?;
    if shape.property_count >= 64 {
        return None; // Dictionary mode threshold
    }
    let new_id = table.transition(from, property_hash);
    drop(table);
    // Log the transition for JIT shape-guard invalidation.
    if let Ok(mut log) = handle.transition_log().lock() {
        log.push((from, new_id));
    }
    Some(new_id)
}

/// Hash a property name string to a u32 for shape transition keys.
///
/// Simple FNV-1a hash truncated to u32.
#[inline]
pub fn hash_property_name(name: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in name.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_shape_exists() {
        let table = ShapeTransitionTable::new();
        let root = table.get_shape(ShapeTransitionTable::root());
        assert_eq!(root.id, ShapeId(0));
        assert!(root.properties.is_empty());
        assert_eq!(root.parent, None);
        assert_eq!(root.property_count, 0);
    }

    #[test]
    fn test_transition_creates_new_shape() {
        let mut table = ShapeTransitionTable::new();
        let child = table.transition(ShapeTransitionTable::root(), 42);
        assert_ne!(child, ShapeTransitionTable::root());

        let shape = table.get_shape(child);
        assert_eq!(shape.properties, vec![42]);
        assert_eq!(shape.property_count, 1);
        assert_eq!(shape.parent, Some(ShapeTransitionTable::root()));
    }

    #[test]
    fn test_transition_deduplication() {
        let mut table = ShapeTransitionTable::new();
        let child1 = table.transition(ShapeTransitionTable::root(), 42);
        let child2 = table.transition(ShapeTransitionTable::root(), 42);
        assert_eq!(child1, child2);
        // Only root + one child should exist
        assert_eq!(table.shape_count(), 2);
    }

    #[test]
    fn test_shape_for_keys() {
        let mut table = ShapeTransitionTable::new();
        let shape_id = table.shape_for_keys(&[10, 20, 30]);
        let shape = table.get_shape(shape_id);
        assert_eq!(shape.properties, vec![10, 20, 30]);
        assert_eq!(shape.property_count, 3);
    }

    #[test]
    fn test_property_index() {
        let mut table = ShapeTransitionTable::new();
        let shape_id = table.shape_for_keys(&[10, 20, 30]);
        assert_eq!(table.property_index(shape_id, 10), Some(0));
        assert_eq!(table.property_index(shape_id, 20), Some(1));
        assert_eq!(table.property_index(shape_id, 30), Some(2));
    }

    #[test]
    fn test_property_index_missing() {
        let mut table = ShapeTransitionTable::new();
        let shape_id = table.shape_for_keys(&[10, 20]);
        assert_eq!(table.property_index(shape_id, 99), None);
    }

    #[test]
    fn test_multiple_transition_paths() {
        let mut table = ShapeTransitionTable::new();
        // Path 1: root -> a -> b
        let ab = table.shape_for_keys(&[1, 2]);
        // Path 2: root -> b -> a
        let ba = table.shape_for_keys(&[2, 1]);
        // Different property orders must produce different shapes
        assert_ne!(ab, ba);
        let shape_ab = table.get_shape(ab);
        let shape_ba = table.get_shape(ba);
        assert_eq!(shape_ab.properties, vec![1, 2]);
        assert_eq!(shape_ba.properties, vec![2, 1]);
    }

    #[test]
    fn test_shape_count() {
        let mut table = ShapeTransitionTable::new();
        assert_eq!(table.shape_count(), 1); // root only
        table.transition(ShapeTransitionTable::root(), 1);
        assert_eq!(table.shape_count(), 2);
        table.transition(ShapeTransitionTable::root(), 2);
        assert_eq!(table.shape_count(), 3);
        // Duplicate transition should not create new shape
        table.transition(ShapeTransitionTable::root(), 1);
        assert_eq!(table.shape_count(), 3);
    }

    #[test]
    fn test_parent_chain() {
        let mut table = ShapeTransitionTable::new();
        let a = table.transition(ShapeTransitionTable::root(), 10);
        let ab = table.transition(a, 20);
        let abc = table.transition(ab, 30);

        let shape_abc = table.get_shape(abc);
        assert_eq!(shape_abc.parent, Some(ab));

        let shape_ab = table.get_shape(ab);
        assert_eq!(shape_ab.parent, Some(a));

        let shape_a = table.get_shape(a);
        assert_eq!(shape_a.parent, Some(ShapeTransitionTable::root()));
    }

    #[test]
    fn test_shape_id_display() {
        assert_eq!(format!("{}", ShapeId(0)), "Shape(0)");
        assert_eq!(format!("{}", ShapeId(42)), "Shape(42)");
    }

    #[test]
    fn test_hash_property_name_consistency() {
        let h1 = hash_property_name("foo");
        let h2 = hash_property_name("foo");
        assert_eq!(h1, h2);
        assert_ne!(hash_property_name("foo"), hash_property_name("bar"));
    }

    #[test]
    fn test_global_shape_for_keys() {
        let handle = crate::shape_graph_current::ShapeTableHandle::new();
        let _scope = crate::shape_graph_current::SyncShapeTableScope::enter(handle);
        let keys = &[hash_property_name("x"), hash_property_name("y")];
        let id1 = shape_for_hashmap_keys(keys).unwrap();
        let id2 = shape_for_hashmap_keys(keys).unwrap();
        assert_eq!(id1, id2); // Same keys → same shape
    }

    #[test]
    fn test_global_shape_transition() {
        let handle = crate::shape_graph_current::ShapeTableHandle::new();
        let _scope = crate::shape_graph_current::SyncShapeTableScope::enter(handle);
        let root = ShapeTransitionTable::root();
        let prop = hash_property_name("test_prop");
        let child = shape_transition(root, prop).unwrap();
        assert_ne!(child, root);
    }

    #[test]
    fn test_dictionary_mode_threshold() {
        let handle = crate::shape_graph_current::ShapeTableHandle::new();
        let _scope = crate::shape_graph_current::SyncShapeTableScope::enter(handle);
        // More than 64 properties → dictionary mode (None)
        let keys: Vec<u32> = (0..65).collect();
        assert!(shape_for_hashmap_keys(&keys).is_none());
    }

    #[test]
    fn test_free_funcs_degrade_without_scope() {
        // Without an active scope the free functions return None / empty,
        // matching the previous lock-poisoning fallback. This keeps tests
        // that indirectly construct HashMapData outside a VM scope
        // (e.g. unit tests for JSON/YAML/CSV decoders) alive.
        assert!(shape_for_hashmap_keys(&[1, 2]).is_none());
        assert!(shape_transition(ShapeId(0), 42).is_none());
        assert!(shape_property_index(ShapeId(0), 42).is_none());
        assert!(drain_shape_transitions().is_empty());
    }

    /// Regression test for the B5 cross-table stale-ShapeId bug: a
    /// `ShapeId` minted under one transition table must not panic when
    /// fed to another. This can happen in real workloads when a
    /// `HashMapData::shape_id` is computed under the process-default
    /// ambient handle (via `ValueWord::from_hashmap_pairs` called
    /// outside any VM scope) and later observed inside a per-VM scope
    /// whose fresh table has no knowledge of that id. Both
    /// `property_index` and `shape_transition` must degrade to `None`
    /// instead of indexing out of bounds.
    #[test]
    fn cross_table_stale_shape_id_degrades_gracefully() {
        let table = ShapeTransitionTable::new();
        let huge = ShapeId(9_999);
        assert_eq!(table.property_index(huge, 42), None);
        assert!(table.try_get_shape(huge).is_none());

        // Free-function path via a fresh scoped handle must also degrade.
        let handle = crate::shape_graph_current::ShapeTableHandle::new();
        let _scope = crate::shape_graph_current::SyncShapeTableScope::enter(handle);
        assert_eq!(shape_property_index(huge, 42), None);
        assert_eq!(shape_transition(huge, 42), None);
    }
}
