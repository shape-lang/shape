//! Module Binding Registry - Single source of truth for module binding values
//!
//! This module provides a unified module binding registry that is shared between
//! the interpreter, VM, and (future) JIT compiler. All module binding values
//! (functions, constants, imported symbols) live here.
//!
//! Design goals:
//! - Name → index mapping for fast compilation
//! - Index → value for O(1) runtime access
//! - Stable memory addresses for JIT compilation
//! - Thread-safe access via RwLock

use crate::Result;
use shape_ast::error::ShapeError;
use shape_value::ValueWord;
use std::collections::HashMap;

/// Single source of truth for all module binding values.
///
/// Used by:
/// - Interpreter: name-based lookup
/// - VM: index-based lookup (after compilation resolves names)
/// - JIT: stable pointers for inlined access
#[derive(Debug)]
pub struct ModuleBindingRegistry {
    /// Name → index mapping (for compilation)
    name_to_index: HashMap<String, u32>,

    /// Index → name mapping (for debugging/errors)
    index_to_name: Vec<String>,

    /// The actual values (NaN-boxed) - accessed by index for O(1) lookup
    values: Vec<ValueWord>,

    /// Track which module bindings are constants (functions, imports)
    is_const: Vec<bool>,
}

impl Default for ModuleBindingRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleBindingRegistry {
    /// Create a new empty module binding registry
    pub fn new() -> Self {
        Self {
            name_to_index: HashMap::new(),
            index_to_name: Vec::new(),
            values: Vec::new(),
            is_const: Vec::new(),
        }
    }

    /// Create with pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            name_to_index: HashMap::with_capacity(capacity),
            index_to_name: Vec::with_capacity(capacity),
            values: Vec::with_capacity(capacity),
            is_const: Vec::with_capacity(capacity),
        }
    }

    /// Register or update a module binding, returns its stable index.
    ///
    /// If the module binding already exists:
    /// - If it's const and we're re-registering with same constness, update value
    /// - If it's const and we're trying to make it mutable, error
    /// - If it's mutable, always update
    ///
    /// # Arguments
    /// * `name` - The module binding's name
    /// * `value` - The value to store (converted to ValueWord internally)
    /// * `is_const` - Whether this module binding is constant (functions, imports)
    ///
    /// # Returns
    /// The stable index for this module binding
    pub fn register(&mut self, name: &str, value: ValueWord, is_const: bool) -> Result<u32> {
        self.register_nb(name, value, is_const)
    }

    /// Register or update a module binding with a ValueWord value, returns its stable index.
    pub fn register_nb(&mut self, name: &str, value: ValueWord, is_const: bool) -> Result<u32> {
        if let Some(&idx) = self.name_to_index.get(name) {
            let idx_usize = idx as usize;

            // Allow re-registration of const module bindings (e.g., during stdlib reload)
            // but don't allow changing const to mutable
            if self.is_const[idx_usize] && !is_const {
                return Err(ShapeError::RuntimeError {
                    message: format!("Cannot redeclare const '{}' as mutable", name),
                    location: None,
                });
            }

            self.values[idx_usize] = value;
            self.is_const[idx_usize] = is_const;
            Ok(idx)
        } else {
            // New module binding
            let idx = self.values.len() as u32;
            self.name_to_index.insert(name.to_string(), idx);
            self.index_to_name.push(name.to_string());
            self.values.push(value);
            self.is_const.push(is_const);
            Ok(idx)
        }
    }

    /// Register a constant module binding (convenience method)
    pub fn register_const(&mut self, name: &str, value: ValueWord) -> Result<u32> {
        self.register(name, value, true)
    }

    /// Register a mutable module binding (convenience method)
    pub fn register_mut(&mut self, name: &str, value: ValueWord) -> Result<u32> {
        self.register_nb(name, value, false)
    }

    /// Check if a module binding exists
    pub fn contains(&self, name: &str) -> bool {
        self.name_to_index.contains_key(name)
    }

    /// Resolve name to index (compile-time)
    pub fn resolve(&self, name: &str) -> Option<u32> {
        self.name_to_index.get(name).copied()
    }

    /// Get name for an index (for error messages)
    pub fn get_name(&self, idx: u32) -> Option<&str> {
        self.index_to_name.get(idx as usize).map(|s| s.as_str())
    }

    /// Get by name as owned ValueWord (interpreter, dynamic lookup)
    pub fn get_by_name(&self, name: &str) -> Option<ValueWord> {
        self.name_to_index
            .get(name)
            .map(|&idx| self.values[idx as usize].clone())
    }

    /// Get by index as ValueWord reference (O(1))
    #[inline]
    pub fn get_by_index(&self, idx: u32) -> Option<&ValueWord> {
        self.values.get(idx as usize)
    }

    /// Set by index from ValueWord (for VM assignment)
    pub fn set_by_index(&mut self, idx: u32, value: ValueWord) -> Result<()> {
        let idx_usize = idx as usize;
        if idx_usize >= self.values.len() {
            return Err(ShapeError::RuntimeError {
                message: format!("module binding index {} out of bounds", idx),
                location: None,
            });
        }
        if self.is_const[idx_usize] {
            return Err(ShapeError::RuntimeError {
                message: format!("Cannot assign to const '{}'", self.index_to_name[idx_usize]),
                location: None,
            });
        }
        self.values[idx_usize] = value;
        Ok(())
    }

    /// Check if a module binding is const
    pub fn is_const(&self, name: &str) -> Option<bool> {
        self.name_to_index
            .get(name)
            .map(|&idx| self.is_const[idx as usize])
    }

    /// Check if a module binding at index is const
    pub fn is_const_by_index(&self, idx: u32) -> Option<bool> {
        self.is_const.get(idx as usize).copied()
    }

    /// Get the number of registered module bindings
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Get all module binding names (for debugging/introspection)
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.index_to_name.iter().map(|s| s.as_str())
    }

    /// Get stable pointer for JIT (address won't change after registration)
    ///
    /// # Safety
    /// The pointer is valid as long as no new module bindings are registered.
    /// For JIT, call this after all module bindings are registered.
    #[inline]
    pub fn get_ptr(&self, idx: u32) -> Option<*const ValueWord> {
        self.values.get(idx as usize).map(|v| v as *const ValueWord)
    }

    /// Snapshot constant module bindings for JIT constant folding
    pub fn snapshot_constants(&self) -> Vec<(u32, ValueWord)> {
        self.values
            .iter()
            .enumerate()
            .filter(|(i, _)| self.is_const[*i])
            .map(|(i, v)| (i as u32, v.clone()))
            .collect()
    }

    /// Clear all module bindings (for testing or reset)
    pub fn clear(&mut self) {
        self.name_to_index.clear();
        self.index_to_name.clear();
        self.values.clear();
        self.is_const.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::ValueWordExt;
    use shape_value::heap_value::HeapValue;

    #[test]
    fn test_register_and_resolve() {
        let mut registry = ModuleBindingRegistry::new();

        let idx = registry
            .register_const("x", ValueWord::from_f64(42.0))
            .unwrap();
        assert_eq!(idx, 0);

        let idx2 = registry
            .register_const("y", ValueWord::from_f64(100.0))
            .unwrap();
        assert_eq!(idx2, 1);

        assert_eq!(registry.resolve("x"), Some(0));
        assert_eq!(registry.resolve("y"), Some(1));
        assert_eq!(registry.resolve("z"), None);
    }

    #[test]
    fn test_get_by_name() {
        let mut registry = ModuleBindingRegistry::new();
        registry
            .register_const("pi", ValueWord::from_f64(3.14159))
            .unwrap();

        let val = registry.get_by_name("pi");
        assert!(val.is_some());
        assert!((val.unwrap().as_f64().unwrap() - 3.14159).abs() < 0.0001);

        assert!(registry.get_by_name("unknown").is_none());
    }

    #[test]
    fn test_get_by_index() {
        let mut registry = ModuleBindingRegistry::new();
        registry
            .register_const("a", ValueWord::from_f64(1.0))
            .unwrap();
        registry
            .register_const("b", ValueWord::from_f64(2.0))
            .unwrap();

        assert_eq!(
            registry.get_by_index(0).and_then(|nb| nb.as_f64()),
            Some(1.0)
        );
        assert_eq!(
            registry.get_by_index(1).and_then(|nb| nb.as_f64()),
            Some(2.0)
        );
        assert!(registry.get_by_index(99).is_none());
    }

    #[test]
    fn test_const_protection() {
        let mut registry = ModuleBindingRegistry::new();
        registry
            .register_const("CONST_VAL", ValueWord::from_f64(42.0))
            .unwrap();

        // Should fail to set const by index
        let result = registry.set_by_index(0, ValueWord::from_f64(100.0));
        assert!(result.is_err());

        // Value should be unchanged
        assert_eq!(
            registry.get_by_index(0).and_then(|nb| nb.as_f64()),
            Some(42.0)
        );
    }

    #[test]
    fn test_mutable_module_binding() {
        let mut registry = ModuleBindingRegistry::new();
        registry
            .register_mut("counter", ValueWord::from_f64(0.0))
            .unwrap();

        // Should succeed to set mutable by index
        registry.set_by_index(0, ValueWord::from_f64(1.0)).unwrap();
        assert_eq!(
            registry.get_by_index(0).and_then(|nb| nb.as_f64()),
            Some(1.0)
        );
    }

    #[test]
    fn test_re_register_const() {
        let mut registry = ModuleBindingRegistry::new();
        registry
            .register_const("func", ValueWord::from_f64(1.0))
            .unwrap();

        // Re-registering same const should update value
        registry
            .register_const("func", ValueWord::from_f64(2.0))
            .unwrap();
        assert_eq!(
            registry.get_by_name("func").and_then(|nb| nb.as_f64()),
            Some(2.0)
        );

        // Index should remain the same
        assert_eq!(registry.resolve("func"), Some(0));
    }

    #[test]
    fn test_snapshot_constants() {
        let mut registry = ModuleBindingRegistry::new();
        registry
            .register_const("a", ValueWord::from_f64(1.0))
            .unwrap();
        registry
            .register_mut("b", ValueWord::from_f64(2.0))
            .unwrap();
        registry
            .register_const("c", ValueWord::from_f64(3.0))
            .unwrap();

        let constants = registry.snapshot_constants();
        assert_eq!(constants.len(), 2); // Only a and c are const

        // Check indices
        let indices: Vec<u32> = constants.iter().map(|(i, _)| *i).collect();
        assert!(indices.contains(&0)); // a
        assert!(indices.contains(&2)); // c
    }

    #[test]
    fn test_closure_registration() {
        let mut registry = ModuleBindingRegistry::new();

        let closure_val = ValueWord::from_heap_value(HeapValue::Closure {
            function_id: 0,
            upvalues: vec![],
        });

        let idx = registry.register_const("test_func", closure_val).unwrap();
        assert_eq!(idx, 0);

        // Should be retrievable
        let val = registry.get_by_name("test_func");
        assert!(
            matches!(val, Some(nb) if nb.as_closure().is_some())
        );
    }

    #[test]
    fn test_contains() {
        let mut registry = ModuleBindingRegistry::new();
        registry
            .register_const("exists", ValueWord::from_f64(1.0))
            .unwrap();

        assert!(registry.contains("exists"));
        assert!(!registry.contains("not_exists"));
    }

    #[test]
    fn test_is_const() {
        let mut registry = ModuleBindingRegistry::new();
        registry
            .register_const("constant", ValueWord::from_f64(1.0))
            .unwrap();
        registry
            .register_mut("mutable", ValueWord::from_f64(2.0))
            .unwrap();

        assert_eq!(registry.is_const("constant"), Some(true));
        assert_eq!(registry.is_const("mutable"), Some(false));
        assert_eq!(registry.is_const("unknown"), None);
    }
}
