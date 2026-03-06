//! Deoptimization tracking for JIT-compiled functions.
//!
//! Tracks which JIT-compiled functions depend on specific shape IDs,
//! so that when a shape transitions (e.g., a HashMap gains a property),
//! all functions that guarded on that shape can be invalidated.

use std::collections::{HashMap, HashSet};

use shape_value::shape_graph::ShapeId;

/// Tracks shape dependencies for JIT-compiled functions.
///
/// When a function is compiled with shape guards (e.g., guarding that an
/// object has shape X for inline caching), the shape IDs it depends on
/// are registered here. When a shape transition occurs, all functions
/// that depend on the transitioning shape are invalidated.
pub struct DeoptTracker {
    /// function_id → set of ShapeIds it depends on
    dependencies: HashMap<u16, HashSet<ShapeId>>,
    /// shape_id → set of function_ids that depend on it
    shape_dependents: HashMap<ShapeId, HashSet<u16>>,
}

impl DeoptTracker {
    /// Create an empty deopt tracker.
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            shape_dependents: HashMap::new(),
        }
    }

    /// Register shape dependencies for a compiled function.
    ///
    /// Called after successful JIT compilation when the compilation result
    /// includes shape guard IDs.
    pub fn register(&mut self, function_id: u16, shape_ids: &[ShapeId]) {
        if shape_ids.is_empty() {
            return;
        }
        let dep_set = self
            .dependencies
            .entry(function_id)
            .or_insert_with(HashSet::new);
        for &sid in shape_ids {
            dep_set.insert(sid);
            self.shape_dependents
                .entry(sid)
                .or_insert_with(HashSet::new)
                .insert(function_id);
        }
    }

    /// Invalidate all functions that depend on the given shape.
    ///
    /// Returns the list of function IDs that were invalidated (need to
    /// have their JIT code removed from the native_code_table).
    pub fn invalidate_shape(&mut self, shape_id: ShapeId) -> Vec<u16> {
        let dependents = match self.shape_dependents.remove(&shape_id) {
            Some(set) => set,
            None => return Vec::new(),
        };

        let mut invalidated = Vec::with_capacity(dependents.len());
        for func_id in dependents {
            // Remove all of this function's dependencies
            if let Some(dep_shapes) = self.dependencies.remove(&func_id) {
                // Clean up reverse mappings for other shapes this function depended on
                for sid in &dep_shapes {
                    if *sid != shape_id {
                        if let Some(funcs) = self.shape_dependents.get_mut(sid) {
                            funcs.remove(&func_id);
                            if funcs.is_empty() {
                                self.shape_dependents.remove(sid);
                            }
                        }
                    }
                }
            }
            invalidated.push(func_id);
        }

        invalidated
    }

    /// Clear all dependencies for a function (e.g., when it's recompiled).
    pub fn clear_function(&mut self, function_id: u16) {
        if let Some(dep_shapes) = self.dependencies.remove(&function_id) {
            for sid in dep_shapes {
                if let Some(funcs) = self.shape_dependents.get_mut(&sid) {
                    funcs.remove(&function_id);
                    if funcs.is_empty() {
                        self.shape_dependents.remove(&sid);
                    }
                }
            }
        }
    }

    /// Number of functions being tracked.
    pub fn tracked_function_count(&self) -> usize {
        self.dependencies.len()
    }

    /// Number of shapes being watched.
    pub fn watched_shape_count(&self) -> usize {
        self.shape_dependents.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_invalidate() {
        let mut tracker = DeoptTracker::new();
        let s1 = ShapeId(1);
        let s2 = ShapeId(2);

        tracker.register(0, &[s1, s2]);
        tracker.register(1, &[s1]);
        assert_eq!(tracker.tracked_function_count(), 2);
        assert_eq!(tracker.watched_shape_count(), 2);

        // Invalidate shape 1 — both functions depend on it
        let invalidated = tracker.invalidate_shape(s1);
        assert_eq!(invalidated.len(), 2);
        assert!(invalidated.contains(&0));
        assert!(invalidated.contains(&1));

        // Both functions fully removed
        assert_eq!(tracker.tracked_function_count(), 0);
        // Shape 2 no longer watched (function 0 was the only dependent)
        assert_eq!(tracker.watched_shape_count(), 0);
    }

    #[test]
    fn test_invalidate_no_dependents() {
        let mut tracker = DeoptTracker::new();
        let invalidated = tracker.invalidate_shape(ShapeId(99));
        assert!(invalidated.is_empty());
    }

    #[test]
    fn test_clear_function() {
        let mut tracker = DeoptTracker::new();
        let s1 = ShapeId(1);
        tracker.register(0, &[s1]);
        tracker.register(1, &[s1]);

        tracker.clear_function(0);
        assert_eq!(tracker.tracked_function_count(), 1);

        // Shape 1 still watched by function 1
        let invalidated = tracker.invalidate_shape(s1);
        assert_eq!(invalidated, vec![1]);
    }

    #[test]
    fn test_register_empty_shapes() {
        let mut tracker = DeoptTracker::new();
        tracker.register(0, &[]);
        assert_eq!(tracker.tracked_function_count(), 0);
    }

    #[test]
    fn test_duplicate_registration() {
        let mut tracker = DeoptTracker::new();
        let s1 = ShapeId(1);
        tracker.register(0, &[s1]);
        tracker.register(0, &[s1]); // duplicate
        assert_eq!(tracker.tracked_function_count(), 1);
        assert_eq!(tracker.watched_shape_count(), 1);
    }

    #[test]
    fn test_invalidate_partial_overlap() {
        let mut tracker = DeoptTracker::new();
        let s1 = ShapeId(1);
        let s2 = ShapeId(2);
        let s3 = ShapeId(3);

        tracker.register(0, &[s1, s2]); // depends on s1, s2
        tracker.register(1, &[s2, s3]); // depends on s2, s3

        // Invalidate s2 — both functions invalidated
        let invalidated = tracker.invalidate_shape(s2);
        assert_eq!(invalidated.len(), 2);

        // All cleaned up
        assert_eq!(tracker.tracked_function_count(), 0);
        assert_eq!(tracker.watched_shape_count(), 0);
    }
}
