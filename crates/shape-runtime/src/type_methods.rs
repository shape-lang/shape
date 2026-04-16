//! Type method registry for storing user-defined methods on types
//!
//! This module provides the infrastructure for storing and retrieving
//! methods that have been added to types via the `extend` statement.

use crate::type_system::annotation_to_string;
use shape_ast::ast::{MethodDef, TypeName};
use shape_value::{ValueWord, ValueWordExt};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Registry for storing type methods
#[derive(Debug, Clone)]
pub struct TypeMethodRegistry {
    /// Methods stored by type name and method name
    /// The Vec allows for method overloading
    methods: Arc<RwLock<HashMap<String, HashMap<String, Vec<MethodDef>>>>>,
}

impl TypeMethodRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            methods: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a method for a type
    pub fn register_method(&self, type_name: &TypeName, method: MethodDef) {
        let mut methods = self.methods.write().unwrap();

        // Get the type name as a string
        let type_str = match type_name {
            TypeName::Simple(name) => name.to_string(),
            TypeName::Generic { name, type_args } => {
                // Convert generic types with their full signature
                // e.g., "Table<Row>", "Vec<Number>"
                if type_args.is_empty() {
                    name.to_string()
                } else {
                    // Convert type arguments to strings
                    let type_arg_strs: Vec<String> =
                        type_args.iter().map(annotation_to_string).collect();
                    format!("{}<{}>", name, type_arg_strs.join(", "))
                }
            }
        };

        // Get or create the method map for this type
        let type_methods = methods.entry(type_str.clone()).or_default();

        // Add the method to the overload list
        type_methods
            .entry(method.name.clone())
            .or_default()
            .push(method);
    }

    /// Get all methods for a type with a given name
    pub fn get_methods(&self, type_name: &str, method_name: &str) -> Option<Vec<MethodDef>> {
        let methods = self.methods.read().unwrap();
        methods
            .get(type_name)
            .and_then(|type_methods| type_methods.get(method_name))
            .cloned()
    }

    /// Get the type name for a value
    pub fn get_value_type_name(value: &ValueWord) -> String {
        value.type_name().to_string()
    }

    /// Get all methods for a type
    pub fn get_all_methods(&self, type_name: &str) -> Vec<MethodDef> {
        let methods = self.methods.read().unwrap();

        methods
            .get(type_name)
            .map(|type_methods| type_methods.values().flatten().cloned().collect())
            .unwrap_or_default()
    }

    /// Check if a type has any methods registered
    pub fn has_type(&self, type_name: &str) -> bool {
        let methods = self.methods.read().unwrap();
        methods.contains_key(type_name)
    }

    /// Get all registered type names
    pub fn get_registered_types(&self) -> Vec<String> {
        let methods = self.methods.read().unwrap();
        methods.keys().cloned().collect()
    }

    /// Get a debug string representation of the registry state
    pub fn debug_state(&self) -> String {
        let methods = self.methods.read().unwrap();
        let mut output = String::new();

        output.push_str("TypeMethodRegistry State:\n");
        output.push_str(&format!("  Total registered types: {}\n", methods.len()));

        if methods.is_empty() {
            output.push_str("  (No types registered)\n");
        } else {
            for (type_name, type_methods) in methods.iter() {
                output.push_str(&format!("  Type: {}\n", type_name));
                for (method_name, overloads) in type_methods.iter() {
                    output.push_str(&format!(
                        "    Method: {} ({} overloads)\n",
                        method_name,
                        overloads.len()
                    ));
                    for (i, overload) in overloads.iter().enumerate() {
                        output.push_str(&format!(
                            "      Overload {}: {} params",
                            i + 1,
                            overload.params.len()
                        ));
                        if overload.when_clause.is_some() {
                            output.push_str(" (with when clause)");
                        }
                        output.push('\n');
                    }
                }
            }
        }

        output
    }
}

impl Default for TypeMethodRegistry {
    fn default() -> Self {
        Self::new()
    }
}
