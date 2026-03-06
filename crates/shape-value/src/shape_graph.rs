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
use std::sync::Mutex;

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
    /// Returns `None` if the property is not part of this shape.
    /// O(n) scan of the shape's properties list.
    pub fn property_index(&self, shape_id: ShapeId, property_name: u32) -> Option<usize> {
        let shape = &self.shapes[shape_id.0 as usize];
        shape.properties.iter().position(|&p| p == property_name)
    }

    /// Total number of shapes in the table.
    #[inline]
    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }
}

// ── Global shape table ──────────────────────────────────────────────────────

static GLOBAL_SHAPE_TABLE: std::sync::LazyLock<Mutex<ShapeTransitionTable>> =
    std::sync::LazyLock::new(|| Mutex::new(ShapeTransitionTable::new()));

// ── Shape transition event log ──────────────────────────────────────────────
//
// Records (parent_shape, new_shape) pairs whenever a shape transition occurs.
// The TierManager drains this buffer periodically to detect shape changes
// that invalidate JIT-compiled shape guards.

static SHAPE_TRANSITION_LOG: std::sync::LazyLock<Mutex<Vec<(ShapeId, ShapeId)>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

/// Drain all pending shape transition events.
///
/// Returns the events accumulated since the last drain. Each event is
/// `(parent_shape_id, new_child_shape_id)`.
///
/// Called by `TierManager::check_shape_invalidations()` during `poll_completions()`.
pub fn drain_shape_transitions() -> Vec<(ShapeId, ShapeId)> {
    SHAPE_TRANSITION_LOG
        .lock()
        .map(|mut log| std::mem::take(&mut *log))
        .unwrap_or_default()
}

/// Compute a ShapeId for a HashMap with the given key hashes (in insertion order).
///
/// Uses the global shape transition table. Returns `None` if the lock is poisoned
/// or if there are more than 64 properties (dictionary mode threshold).
pub fn shape_for_hashmap_keys(key_hashes: &[u32]) -> Option<ShapeId> {
    if key_hashes.len() > 64 {
        return None; // Dictionary mode: too many properties
    }
    let mut table = GLOBAL_SHAPE_TABLE.lock().ok()?;
    Some(table.shape_for_keys(key_hashes))
}

/// Look up the slot index of a property in a shape.
///
/// Uses the global shape transition table.
pub fn shape_property_index(shape_id: ShapeId, property_hash: u32) -> Option<usize> {
    let table = GLOBAL_SHAPE_TABLE.lock().ok()?;
    table.property_index(shape_id, property_hash)
}

/// Transition a shape by adding a new property.
///
/// Uses the global shape transition table. Returns `None` if dictionary mode
/// threshold (>64 properties) would be exceeded.
pub fn shape_transition(from: ShapeId, property_hash: u32) -> Option<ShapeId> {
    let mut table = GLOBAL_SHAPE_TABLE.lock().ok()?;
    let shape = table.get_shape(from);
    if shape.property_count >= 64 {
        return None; // Dictionary mode threshold
    }
    let new_id = table.transition(from, property_hash);
    // Log the transition for JIT shape guard invalidation
    if let Ok(mut log) = SHAPE_TRANSITION_LOG.lock() {
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
        let keys = &[hash_property_name("x"), hash_property_name("y")];
        let id1 = shape_for_hashmap_keys(keys).unwrap();
        let id2 = shape_for_hashmap_keys(keys).unwrap();
        assert_eq!(id1, id2); // Same keys → same shape
    }

    #[test]
    fn test_global_shape_transition() {
        let root = ShapeTransitionTable::root();
        let prop = hash_property_name("test_prop");
        let child = shape_transition(root, prop).unwrap();
        assert_ne!(child, root);
    }

    #[test]
    fn test_dictionary_mode_threshold() {
        // More than 64 properties → dictionary mode (None)
        let keys: Vec<u32> = (0..65).collect();
        assert!(shape_for_hashmap_keys(&keys).is_none());
    }
}
