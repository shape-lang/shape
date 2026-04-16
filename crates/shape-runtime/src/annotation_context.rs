//! Annotation context and decorator registry
//!
//! This module provides the infrastructure for Shape's annotation system.
//!
//! ## Design Philosophy
//!
//! Annotations in Shape are **fully defined in Shape stdlib**, not hardcoded in Rust.
//! This module provides the generic runtime primitives that annotation lifecycle hooks use.
//!
//! ## Annotation Lifecycle Hooks
//!
//! Annotations can define handlers for different lifecycle events:
//! - `on_define(fn, ctx)` - Called when function is first defined
//! - `before(fn, args, ctx)` - Called before each function invocation
//! - `after(fn, args, result, ctx)` - Called after each function invocation
//! - `metadata()` - Static metadata for tooling and optimization
//!
//! ## Example (stdlib/finance/annotations/pattern.shape)
//!
//! ```shape
//! annotation pattern() {
//!     on_define(fn, ctx) {
//!         ctx.registry("patterns").set(fn.name, fn);
//!     }
//!     metadata() { return { is_pattern: true }; }
//! }
//! ```
//!
//! ## Runtime Primitives
//!
//! The `AnnotationContext` provides domain-agnostic primitives:
//! - `cache` - Key-value cache for memoization
//! - `state` - Per-annotation persistent state
//! - `registry(name)` - Named registries (patterns, strategies, features, etc.)
//! - `emit(event, data)` - Event emission for alerts, logging
//! - `data` - Data range manipulation (extend/restore for warmup)

use shape_value::ValueWord;
use std::collections::HashMap;

// ============================================================================
// Annotation Registry
// ============================================================================

/// Registry for annotation definitions
///
/// Stores `annotation ... { ... }` definitions that can be looked up by name.
/// These definitions include lifecycle hooks (on_define, before, after, metadata).
#[derive(Clone)]
pub struct AnnotationRegistry {
    annotations: HashMap<String, shape_ast::ast::AnnotationDef>,
}

impl AnnotationRegistry {
    pub fn new() -> Self {
        Self {
            annotations: HashMap::new(),
        }
    }

    pub fn register(&mut self, def: shape_ast::ast::AnnotationDef) {
        self.annotations.insert(def.name.clone(), def);
    }

    pub fn get(&self, name: &str) -> Option<&shape_ast::ast::AnnotationDef> {
        self.annotations.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.annotations.contains_key(name)
    }
}

impl Default for AnnotationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Annotation Context - Runtime Primitives
// ============================================================================

/// Context passed to annotation lifecycle hooks
///
/// Provides domain-agnostic primitives that annotation handlers use.
/// This is the `ctx` parameter in handlers like `on_define(fn, ctx)`.
#[derive(Debug, Clone)]
pub struct AnnotationContext {
    /// Key-value cache for memoization (e.g., @cached, @indicator)
    cache: AnnotationCache,
    /// Per-annotation persistent state
    state: AnnotationState,
    /// Named registries (patterns, strategies, features, etc.)
    registries: HashMap<String, NamedRegistry>,
    /// Emitted events (for @alert, @logged annotations)
    events: Vec<EmittedEvent>,
    /// Data range manipulation state (for @warmup)
    data_range: DataRangeState,
}

impl AnnotationContext {
    pub fn new() -> Self {
        Self {
            cache: AnnotationCache::new(),
            state: AnnotationState::new(),
            registries: HashMap::new(),
            events: Vec::new(),
            data_range: DataRangeState::new(),
        }
    }

    /// Get the cache for memoization
    pub fn cache(&self) -> &AnnotationCache {
        &self.cache
    }

    /// Get mutable cache for memoization
    pub fn cache_mut(&mut self) -> &mut AnnotationCache {
        &mut self.cache
    }

    /// Get the per-annotation state
    pub fn state(&self) -> &AnnotationState {
        &self.state
    }

    /// Get mutable per-annotation state
    pub fn state_mut(&mut self) -> &mut AnnotationState {
        &mut self.state
    }

    /// Get or create a named registry
    pub fn registry(&mut self, name: &str) -> &mut NamedRegistry {
        self.registries.entry(name.to_string()).or_default()
    }

    /// Emit an event (for alerts, logging, etc.)
    pub fn emit(&mut self, event_type: &str, data: ValueWord) {
        self.events.push(EmittedEvent {
            event_type: event_type.to_string(),
            data,
            timestamp: std::time::Instant::now(),
        });
    }

    /// Get all emitted events
    pub fn events(&self) -> &[EmittedEvent] {
        &self.events
    }

    /// Clear emitted events
    pub fn clear_events(&mut self) {
        self.events.clear();
    }

    /// Get data range manipulation state
    pub fn data_range(&self) -> &DataRangeState {
        &self.data_range
    }

    /// Get mutable data range manipulation state
    pub fn data_range_mut(&mut self) -> &mut DataRangeState {
        &mut self.data_range
    }
}

impl Default for AnnotationContext {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Cache Primitive
// ============================================================================

/// Key-value cache for annotation memoization
///
/// Used by annotations like @cached, @indicator, @memo
#[derive(Debug, Clone, Default)]
pub struct AnnotationCache {
    entries: HashMap<String, CacheEntry>,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub value: ValueWord,
    pub created_at: std::time::Instant,
}

impl AnnotationCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Get a cached value by key as ValueWord reference
    pub fn get(&self, key: &str) -> Option<&ValueWord> {
        self.entries.get(key).map(|e| &e.value)
    }

    /// Get a cached entry (includes metadata)
    pub fn get_entry(&self, key: &str) -> Option<&CacheEntry> {
        self.entries.get(key)
    }

    /// Set a cached value
    pub fn set(&mut self, key: String, value: ValueWord) {
        self.entries.insert(
            key,
            CacheEntry {
                value,
                created_at: std::time::Instant::now(),
            },
        );
    }

    /// Check if key exists
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Remove a cached value
    pub fn remove(&mut self, key: &str) -> Option<ValueWord> {
        self.entries.remove(key).map(|e| e.value)
    }

    /// Clear all cached values
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

// ============================================================================
// State Primitive
// ============================================================================

/// Per-annotation persistent state
///
/// Used for annotations that need to maintain state across calls
#[derive(Debug, Clone, Default)]
pub struct AnnotationState {
    values: HashMap<String, ValueWord>,
}

impl AnnotationState {
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    /// Get a state value as ValueWord reference
    pub fn get(&self, key: &str) -> Option<&ValueWord> {
        self.values.get(key)
    }

    pub fn set(&mut self, key: String, value: ValueWord) {
        self.values.insert(key, value);
    }

    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    pub fn remove(&mut self, key: &str) -> Option<ValueWord> {
        self.values.remove(key)
    }

    pub fn clear(&mut self) {
        self.values.clear();
    }
}

// ============================================================================
// Registry Primitive
// ============================================================================

/// A named registry for storing values (functions, patterns, etc.)
///
/// Used by custom annotations (e.g. @strategy, @feature)
#[derive(Debug, Clone, Default)]
pub struct NamedRegistry {
    entries: HashMap<String, ValueWord>,
}

impl NamedRegistry {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Get a registry value as ValueWord reference
    pub fn get(&self, key: &str) -> Option<&ValueWord> {
        self.entries.get(key)
    }

    pub fn set(&mut self, key: String, value: ValueWord) {
        self.entries.insert(key, value);
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn remove(&mut self, key: &str) -> Option<ValueWord> {
        self.entries.remove(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }

    /// Iterate over values as ValueWord references
    pub fn values(&self) -> impl Iterator<Item = &ValueWord> {
        self.entries.values()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ============================================================================
// Event Emission Primitive
// ============================================================================

/// An emitted event from an annotation handler
///
/// Used by annotations like @alert, @logged, @audit_logged
#[derive(Debug, Clone)]
pub struct EmittedEvent {
    pub event_type: String,
    pub data: ValueWord,
    pub timestamp: std::time::Instant,
}

// ============================================================================
// Data Range Manipulation
// ============================================================================

/// State for data range manipulation (used by @warmup)
///
/// Allows annotations to extend the data range (e.g., for warmup periods)
/// and then restore it after processing.
#[derive(Debug, Clone, Default)]
pub struct DataRangeState {
    /// Original data range start (if extended)
    original_start: Option<usize>,
    /// Original data range end (if extended)
    original_end: Option<usize>,
    /// Amount the range was extended
    extension_amount: Option<usize>,
}

impl DataRangeState {
    pub fn new() -> Self {
        Self {
            original_start: None,
            original_end: None,
            extension_amount: None,
        }
    }

    /// Record the original range before extending
    pub fn save_original(&mut self, start: usize, end: usize) {
        self.original_start = Some(start);
        self.original_end = Some(end);
    }

    /// Record how much the range was extended
    pub fn set_extension(&mut self, amount: usize) {
        self.extension_amount = Some(amount);
    }

    /// Get the original start position
    pub fn original_start(&self) -> Option<usize> {
        self.original_start
    }

    /// Get the original end position
    pub fn original_end(&self) -> Option<usize> {
        self.original_end
    }

    /// Get the extension amount
    pub fn extension_amount(&self) -> Option<usize> {
        self.extension_amount
    }

    /// Check if range is currently extended
    pub fn is_extended(&self) -> bool {
        self.extension_amount.is_some()
    }

    /// Clear the saved state (after restoring)
    pub fn clear(&mut self) {
        self.original_start = None;
        self.original_end = None;
        self.extension_amount = None;
    }
}

// ============================================================================
// Annotation Processor
// ============================================================================
