//! Structural code search over content-addressed function blobs.
//!
//! Enables querying functions by signature, dependency patterns,
//! instruction patterns, and structural properties.

use std::collections::{HashMap, HashSet};
use shape_value::ValueWordExt;

/// A searchable index over a set of function blobs.
pub struct CodeIndex {
    /// All indexed functions by hash.
    functions: HashMap<[u8; 32], IndexedFunction>,
    /// Functions indexed by arity.
    by_arity: HashMap<u16, Vec<[u8; 32]>>,
    /// Functions indexed by callee (dependency).
    by_callee: HashMap<[u8; 32], Vec<[u8; 32]>>,
    /// Functions indexed by referenced type schema.
    by_type_schema: HashMap<String, Vec<[u8; 32]>>,
    /// Functions indexed by name.
    by_name: HashMap<String, [u8; 32]>,
}

/// Indexed metadata about a function.
#[derive(Debug, Clone)]
pub struct IndexedFunction {
    pub hash: [u8; 32],
    pub name: String,
    pub arity: u16,
    pub instruction_count: usize,
    pub dependencies: Vec<[u8; 32]>,
    pub type_schemas: Vec<String>,
    pub is_async: bool,
    pub is_closure: bool,
    pub has_captures: bool,
}

/// Query for searching functions.
#[derive(Debug, Clone, Default)]
pub struct FunctionQuery {
    pub name_pattern: Option<String>,
    pub arity: Option<u16>,
    pub min_instructions: Option<usize>,
    pub max_instructions: Option<usize>,
    pub calls_function: Option<[u8; 32]>,
    pub uses_type: Option<String>,
    pub is_async: Option<bool>,
    pub is_closure: Option<bool>,
}

/// Result of a code search query.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub matches: Vec<IndexedFunction>,
    pub total_indexed: usize,
}

impl CodeIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            by_arity: HashMap::new(),
            by_callee: HashMap::new(),
            by_type_schema: HashMap::new(),
            by_name: HashMap::new(),
        }
    }

    /// Add a function to the index.
    #[allow(clippy::too_many_arguments)]
    pub fn index_function(
        &mut self,
        hash: [u8; 32],
        name: String,
        arity: u16,
        instruction_count: usize,
        dependencies: Vec<[u8; 32]>,
        type_schemas: Vec<String>,
        is_async: bool,
        is_closure: bool,
        captures_count: u16,
    ) {
        let func = IndexedFunction {
            hash,
            name: name.clone(),
            arity,
            instruction_count,
            dependencies: dependencies.clone(),
            type_schemas: type_schemas.clone(),
            is_async,
            is_closure,
            has_captures: captures_count > 0,
        };

        // Index by arity.
        self.by_arity.entry(arity).or_default().push(hash);

        // Index by callee (each dependency is a function this blob calls).
        for dep in &dependencies {
            self.by_callee.entry(*dep).or_default().push(hash);
        }

        // Index by type schema.
        for schema in &type_schemas {
            self.by_type_schema
                .entry(schema.clone())
                .or_default()
                .push(hash);
        }

        // Index by name.
        self.by_name.insert(name, hash);

        // Store the function.
        self.functions.insert(hash, func);
    }

    /// Execute a query against the index, returning all functions that match
    /// every specified criterion.
    pub fn search(&self, query: &FunctionQuery) -> SearchResult {
        let total_indexed = self.functions.len();

        // Start with candidate sets from indexed lookups, then intersect.
        let mut candidates: Option<HashSet<[u8; 32]>> = None;

        // Narrow by arity if specified.
        if let Some(arity) = query.arity {
            let set: HashSet<[u8; 32]> = self
                .by_arity
                .get(&arity)
                .map(|v| v.iter().copied().collect())
                .unwrap_or_default();
            candidates = Some(match candidates {
                Some(c) => c.intersection(&set).copied().collect(),
                None => set,
            });
        }

        // Narrow by callee if specified.
        if let Some(ref callee) = query.calls_function {
            let set: HashSet<[u8; 32]> = self
                .by_callee
                .get(callee)
                .map(|v| v.iter().copied().collect())
                .unwrap_or_default();
            candidates = Some(match candidates {
                Some(c) => c.intersection(&set).copied().collect(),
                None => set,
            });
        }

        // Narrow by type schema if specified.
        if let Some(ref type_name) = query.uses_type {
            let set: HashSet<[u8; 32]> = self
                .by_type_schema
                .get(type_name)
                .map(|v| v.iter().copied().collect())
                .unwrap_or_default();
            candidates = Some(match candidates {
                Some(c) => c.intersection(&set).copied().collect(),
                None => set,
            });
        }

        // If no indexed filter was applied, start with all functions.
        let candidate_iter: Box<dyn Iterator<Item = &IndexedFunction>> = match candidates {
            Some(ref set) => Box::new(set.iter().filter_map(|h| self.functions.get(h))),
            None => Box::new(self.functions.values()),
        };

        // Apply remaining filters that require scanning.
        let matches: Vec<IndexedFunction> = candidate_iter
            .filter(|f| {
                if let Some(ref pattern) = query.name_pattern {
                    if !f.name.contains(pattern.as_str()) {
                        return false;
                    }
                }
                if let Some(min) = query.min_instructions {
                    if f.instruction_count < min {
                        return false;
                    }
                }
                if let Some(max) = query.max_instructions {
                    if f.instruction_count > max {
                        return false;
                    }
                }
                if let Some(want_async) = query.is_async {
                    if f.is_async != want_async {
                        return false;
                    }
                }
                if let Some(want_closure) = query.is_closure {
                    if f.is_closure != want_closure {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        SearchResult {
            matches,
            total_indexed,
        }
    }

    /// Find functions that call the given function hash (i.e. have it as a dependency).
    pub fn find_callers(&self, function_hash: [u8; 32]) -> Vec<IndexedFunction> {
        self.by_callee
            .get(&function_hash)
            .map(|hashes| {
                hashes
                    .iter()
                    .filter_map(|h| self.functions.get(h).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Find functions called by the given function hash (its direct dependencies).
    pub fn find_callees(&self, function_hash: [u8; 32]) -> Vec<IndexedFunction> {
        self.functions
            .get(&function_hash)
            .map(|f| {
                f.dependencies
                    .iter()
                    .filter_map(|h| self.functions.get(h).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Compute the transitive dependency depth for a function.
    ///
    /// Returns `None` if the function is not in the index.
    /// A function with no dependencies has depth 0.
    /// A function that calls only leaf functions has depth 1, etc.
    ///
    /// Cycles are detected and treated as already-visited (depth 0 contribution).
    pub fn dependency_depth(&self, function_hash: [u8; 32]) -> Option<usize> {
        let func = self.functions.get(&function_hash)?;
        if func.dependencies.is_empty() {
            return Some(0);
        }

        // BFS with memoization.
        let mut memo: HashMap<[u8; 32], usize> = HashMap::new();
        Some(self.compute_depth(function_hash, &mut memo, &mut HashSet::new()))
    }

    /// Recursive depth computation with cycle detection.
    fn compute_depth(
        &self,
        hash: [u8; 32],
        memo: &mut HashMap<[u8; 32], usize>,
        visiting: &mut HashSet<[u8; 32]>,
    ) -> usize {
        if let Some(&cached) = memo.get(&hash) {
            return cached;
        }
        if visiting.contains(&hash) {
            // Cycle detected; treat as depth 0 to break the loop.
            return 0;
        }

        let deps = match self.functions.get(&hash) {
            Some(f) => &f.dependencies,
            None => {
                memo.insert(hash, 0);
                return 0;
            }
        };

        if deps.is_empty() {
            memo.insert(hash, 0);
            return 0;
        }

        visiting.insert(hash);
        let max_child = deps
            .iter()
            .map(|d| self.compute_depth(*d, memo, visiting))
            .max()
            .unwrap_or(0);
        visiting.remove(&hash);

        let depth = max_child + 1;
        memo.insert(hash, depth);
        depth
    }

    /// Return the total number of indexed functions.
    pub fn len(&self) -> usize {
        self.functions.len()
    }

    /// Return whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }
}

impl Default for CodeIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ── FunctionQuery builder methods ──────────────────────────────────────────

impl FunctionQuery {
    /// Create a new empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter by name substring.
    pub fn with_name_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.name_pattern = Some(pattern.into());
        self
    }

    /// Filter by exact arity.
    pub fn with_arity(mut self, arity: u16) -> Self {
        self.arity = Some(arity);
        self
    }

    /// Filter by minimum instruction count.
    pub fn with_min_instructions(mut self, min: usize) -> Self {
        self.min_instructions = Some(min);
        self
    }

    /// Filter by maximum instruction count.
    pub fn with_max_instructions(mut self, max: usize) -> Self {
        self.max_instructions = Some(max);
        self
    }

    /// Filter to functions that call a specific function hash.
    pub fn with_calls_function(mut self, hash: [u8; 32]) -> Self {
        self.calls_function = Some(hash);
        self
    }

    /// Filter to functions that reference a type schema.
    pub fn with_uses_type(mut self, type_name: impl Into<String>) -> Self {
        self.uses_type = Some(type_name.into());
        self
    }

    /// Filter by async status.
    pub fn with_async(mut self, is_async: bool) -> Self {
        self.is_async = Some(is_async);
        self
    }

    /// Filter by closure status.
    pub fn with_closure(mut self, is_closure: bool) -> Self {
        self.is_closure = Some(is_closure);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hash(seed: u8) -> [u8; 32] {
        let mut h = [0u8; 32];
        h[0] = seed;
        h
    }

    fn build_test_index() -> CodeIndex {
        let mut idx = CodeIndex::new();

        // Leaf function: add(a, b) - sync, not a closure
        idx.index_function(
            make_hash(1),
            "add".into(),
            2,
            5,
            vec![],
            vec![],
            false,
            false,
            0,
        );

        // Leaf function: mul(a, b) - sync
        idx.index_function(
            make_hash(2),
            "mul".into(),
            2,
            4,
            vec![],
            vec!["Number".into()],
            false,
            false,
            0,
        );

        // Calls add: sum_and_mul(a, b, c) - sync
        idx.index_function(
            make_hash(3),
            "sum_and_mul".into(),
            3,
            12,
            vec![make_hash(1), make_hash(2)],
            vec!["Number".into()],
            false,
            false,
            0,
        );

        // Async closure that captures: fetch_data() - async, closure
        idx.index_function(
            make_hash(4),
            "fetch_data".into(),
            0,
            20,
            vec![make_hash(3)],
            vec!["DataRow".into()],
            true,
            true,
            2,
        );

        // Deep chain: orchestrate() calls fetch_data
        idx.index_function(
            make_hash(5),
            "orchestrate".into(),
            1,
            30,
            vec![make_hash(4)],
            vec![],
            true,
            false,
            0,
        );

        idx
    }

    #[test]
    fn test_new_index_is_empty() {
        let idx = CodeIndex::new();
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn test_index_function_and_len() {
        let idx = build_test_index();
        assert_eq!(idx.len(), 5);
        assert!(!idx.is_empty());
    }

    #[test]
    fn test_search_by_arity() {
        let idx = build_test_index();
        let result = idx.search(&FunctionQuery::new().with_arity(2));
        assert_eq!(result.matches.len(), 2);
        let names: HashSet<&str> = result.matches.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains("add"));
        assert!(names.contains("mul"));
        assert_eq!(result.total_indexed, 5);
    }

    #[test]
    fn test_search_by_name_pattern() {
        let idx = build_test_index();
        let result = idx.search(&FunctionQuery::new().with_name_pattern("mul"));
        assert_eq!(result.matches.len(), 2); // "mul" and "sum_and_mul"
    }

    #[test]
    fn test_search_by_async() {
        let idx = build_test_index();
        let result = idx.search(&FunctionQuery::new().with_async(true));
        assert_eq!(result.matches.len(), 2);
        let names: HashSet<&str> = result.matches.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains("fetch_data"));
        assert!(names.contains("orchestrate"));
    }

    #[test]
    fn test_search_by_closure() {
        let idx = build_test_index();
        let result = idx.search(&FunctionQuery::new().with_closure(true));
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].name, "fetch_data");
        assert!(result.matches[0].has_captures);
    }

    #[test]
    fn test_search_by_calls_function() {
        let idx = build_test_index();
        let result = idx.search(&FunctionQuery::new().with_calls_function(make_hash(1)));
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].name, "sum_and_mul");
    }

    #[test]
    fn test_search_by_uses_type() {
        let idx = build_test_index();
        let result = idx.search(&FunctionQuery::new().with_uses_type("Number"));
        assert_eq!(result.matches.len(), 2);
        let names: HashSet<&str> = result.matches.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains("mul"));
        assert!(names.contains("sum_and_mul"));
    }

    #[test]
    fn test_search_combined_filters() {
        let idx = build_test_index();
        let result = idx.search(&FunctionQuery::new().with_arity(2).with_uses_type("Number"));
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].name, "mul");
    }

    #[test]
    fn test_search_instruction_range() {
        let idx = build_test_index();
        let result = idx.search(
            &FunctionQuery::new()
                .with_min_instructions(10)
                .with_max_instructions(25),
        );
        assert_eq!(result.matches.len(), 2);
        let names: HashSet<&str> = result.matches.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains("sum_and_mul"));
        assert!(names.contains("fetch_data"));
    }

    #[test]
    fn test_search_no_matches() {
        let idx = build_test_index();
        let result = idx.search(&FunctionQuery::new().with_arity(99));
        assert!(result.matches.is_empty());
        assert_eq!(result.total_indexed, 5);
    }

    #[test]
    fn test_find_callers() {
        let idx = build_test_index();
        // Who calls add (hash 1)?
        let callers = idx.find_callers(make_hash(1));
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].name, "sum_and_mul");

        // Who calls sum_and_mul (hash 3)?
        let callers = idx.find_callers(make_hash(3));
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].name, "fetch_data");
    }

    #[test]
    fn test_find_callers_none() {
        let idx = build_test_index();
        // orchestrate (hash 5) is not called by anyone in the index.
        let callers = idx.find_callers(make_hash(5));
        assert!(callers.is_empty());
    }

    #[test]
    fn test_find_callees() {
        let idx = build_test_index();
        // sum_and_mul calls add and mul.
        let callees = idx.find_callees(make_hash(3));
        assert_eq!(callees.len(), 2);
        let names: HashSet<&str> = callees.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains("add"));
        assert!(names.contains("mul"));
    }

    #[test]
    fn test_find_callees_leaf() {
        let idx = build_test_index();
        let callees = idx.find_callees(make_hash(1));
        assert!(callees.is_empty());
    }

    #[test]
    fn test_dependency_depth_leaf() {
        let idx = build_test_index();
        assert_eq!(idx.dependency_depth(make_hash(1)), Some(0));
        assert_eq!(idx.dependency_depth(make_hash(2)), Some(0));
    }

    #[test]
    fn test_dependency_depth_one_level() {
        let idx = build_test_index();
        // sum_and_mul -> {add, mul}, both depth 0 => depth 1
        assert_eq!(idx.dependency_depth(make_hash(3)), Some(1));
    }

    #[test]
    fn test_dependency_depth_two_levels() {
        let idx = build_test_index();
        // fetch_data -> sum_and_mul (depth 1) => depth 2
        assert_eq!(idx.dependency_depth(make_hash(4)), Some(2));
    }

    #[test]
    fn test_dependency_depth_three_levels() {
        let idx = build_test_index();
        // orchestrate -> fetch_data (depth 2) => depth 3
        assert_eq!(idx.dependency_depth(make_hash(5)), Some(3));
    }

    #[test]
    fn test_dependency_depth_unknown_hash() {
        let idx = build_test_index();
        assert_eq!(idx.dependency_depth(make_hash(99)), None);
    }

    #[test]
    fn test_dependency_depth_cycle() {
        let mut idx = CodeIndex::new();
        // a -> b -> a (cycle)
        idx.index_function(
            make_hash(10),
            "a".into(),
            0,
            1,
            vec![make_hash(11)],
            vec![],
            false,
            false,
            0,
        );
        idx.index_function(
            make_hash(11),
            "b".into(),
            0,
            1,
            vec![make_hash(10)],
            vec![],
            false,
            false,
            0,
        );
        // Should not hang; cycle breaks to 0.
        let depth = idx.dependency_depth(make_hash(10));
        assert!(depth.is_some());
        assert!(depth.unwrap() <= 2);
    }

    #[test]
    fn test_function_query_builder_chain() {
        let q = FunctionQuery::new()
            .with_name_pattern("foo")
            .with_arity(3)
            .with_min_instructions(5)
            .with_max_instructions(100)
            .with_async(true)
            .with_closure(false)
            .with_uses_type("Bar");

        assert_eq!(q.name_pattern.as_deref(), Some("foo"));
        assert_eq!(q.arity, Some(3));
        assert_eq!(q.min_instructions, Some(5));
        assert_eq!(q.max_instructions, Some(100));
        assert_eq!(q.is_async, Some(true));
        assert_eq!(q.is_closure, Some(false));
        assert_eq!(q.uses_type.as_deref(), Some("Bar"));
    }
}
