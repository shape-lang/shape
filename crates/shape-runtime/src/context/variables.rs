//! Variable management for ExecutionContext
//!
//! This module handles variable storage, scoping, and pattern destructuring.

use shape_ast::ast::VarKind;
use shape_ast::error::{Result, ShapeError};
use shape_value::{ValueWord, ValueWordExt};
use std::collections::HashMap;
use std::sync::Arc;

/// A variable in the execution context
#[derive(Debug, Clone)]
pub struct Variable {
    /// The variable's current value (NaN-boxed for compact 8-byte storage)
    pub value: ValueWord,
    /// The variable kind (let, var, const)
    pub kind: VarKind,
    /// Whether the variable has been initialized
    pub is_initialized: bool,
    /// Whether this is a function-scoped variable (var, Flexible ownership)
    /// vs block-scoped (let/const, Owned{Immutable,Mutable} ownership)
    pub is_function_scoped: bool,
    /// Optional format hint for display (e.g., "Percent" for meta lookup)
    pub format_hint: Option<String>,
    /// Optional format parameter overrides from type alias (e.g., { decimals: 4 } from type Percent4 = Percent { decimals: 4 })
    pub format_overrides: Option<HashMap<String, ValueWord>>,
}

impl Variable {
    /// Create a new variable
    pub fn new(kind: VarKind, value: Option<ValueWord>) -> Self {
        Self::with_format(kind, value, None, None)
    }

    /// Create a new variable with format hint and parameter overrides
    pub fn with_format(
        kind: VarKind,
        value: Option<ValueWord>,
        format_hint: Option<String>,
        format_overrides: Option<HashMap<String, ValueWord>>,
    ) -> Self {
        let is_function_scoped = matches!(kind, VarKind::Var);
        let (value, is_initialized) = match value {
            Some(v) => (v, true),
            None => (ValueWord::none(), false),
        };

        Self {
            value,
            kind,
            is_initialized,
            is_function_scoped,
            format_hint,
            format_overrides,
        }
    }

    /// Check if this variable can be assigned to
    pub fn can_assign(&self) -> bool {
        match self.kind {
            VarKind::Const => !self.is_initialized, // const can only be assigned during initialization
            VarKind::Let | VarKind::Var => true,
        }
    }

    /// Assign a value to this variable
    pub fn assign(&mut self, value: ValueWord) -> Result<()> {
        if !self.can_assign() {
            return Err(ShapeError::RuntimeError {
                message: "Cannot assign to const variable after initialization".to_string(),
                location: None,
            });
        }

        self.value = value;
        self.is_initialized = true;
        Ok(())
    }

    /// Get the value as ValueWord reference, checking initialization
    pub fn get_value(&self) -> Result<&ValueWord> {
        if !self.is_initialized {
            return Err(ShapeError::RuntimeError {
                message: "Variable used before initialization".to_string(),
                location: None,
            });
        }
        Ok(&self.value)
    }
}

impl super::ExecutionContext {
    /// Set a variable value (for simple assignment without declaration)
    pub fn set_variable(&mut self, name: &str, value: ValueWord) -> Result<()> {
        self.set_variable_nb(name, value)
    }

    /// Set a variable value from ValueWord (avoids ValueWord conversion)
    pub fn set_variable_nb(&mut self, name: &str, value: ValueWord) -> Result<()> {
        // Search from innermost to outermost scope for existing variable
        for scope in self.variable_scopes.iter_mut().rev() {
            if let Some(variable) = scope.get_mut(name) {
                return variable.assign(value);
            }
        }

        // If variable doesn't exist, create a new 'var' variable in current scope
        if let Some(scope) = self.variable_scopes.last_mut() {
            let variable = Variable::new(VarKind::Var, Some(value));
            scope.insert(name.to_string(), variable);
            Ok(())
        } else {
            Err(ShapeError::RuntimeError {
                message: "No scope available for variable assignment".to_string(),
                location: None,
            })
        }
    }

    /// Get a variable value as ValueWord
    pub fn get_variable(&self, name: &str) -> Result<Option<ValueWord>> {
        // Search from innermost to outermost scope
        for scope in self.variable_scopes.iter().rev() {
            if let Some(variable) = scope.get(name) {
                return Ok(Some(variable.get_value()?.clone()));
            }
        }
        Ok(None)
    }

    /// Get a variable value as ValueWord (avoids ValueWord materialization)
    pub fn get_variable_nb(&self, name: &str) -> Result<Option<ValueWord>> {
        for scope in self.variable_scopes.iter().rev() {
            if let Some(variable) = scope.get(name) {
                return Ok(Some(variable.get_value()?.clone()));
            }
        }
        Ok(None)
    }

    /// Declare a new variable (with let, var, const)
    pub fn declare_variable(
        &mut self,
        name: &str,
        kind: VarKind,
        value: Option<ValueWord>,
    ) -> Result<()> {
        self.declare_variable_with_format(name, kind, value, None, None)
    }

    /// Declare a new variable with format hint and parameter overrides
    ///
    /// This is the full version that supports type aliases with meta parameter overrides,
    /// e.g., `type Percent4 = Percent { decimals: 4 }` would store:
    /// - format_hint: Some("Percent")
    /// - format_overrides: Some({ "decimals": 4 })
    pub fn declare_variable_with_format(
        &mut self,
        name: &str,
        kind: VarKind,
        value: Option<ValueWord>,
        format_hint: Option<String>,
        format_overrides: Option<HashMap<String, ValueWord>>,
    ) -> Result<()> {
        // Check if variable already exists in current scope
        if let Some(current_scope) = self.variable_scopes.last() {
            if current_scope.contains_key(name) {
                return Err(ShapeError::RuntimeError {
                    message: format!("Variable '{}' already declared in current scope", name),
                    location: None,
                });
            }
        }

        // const variables must be initialized
        if matches!(kind, VarKind::Const) && value.is_none() {
            return Err(ShapeError::RuntimeError {
                message: format!("const variable '{}' must be initialized", name),
                location: None,
            });
        }

        // Add to current scope
        if let Some(scope) = self.variable_scopes.last_mut() {
            let variable = Variable::with_format(kind, value, format_hint, format_overrides);
            scope.insert(name.to_string(), variable);
            Ok(())
        } else {
            Err(ShapeError::RuntimeError {
                message: "No scope available for variable declaration".to_string(),
                location: None,
            })
        }
    }

    /// Get the format hint for a variable (if any)
    pub fn get_variable_format_hint(&self, name: &str) -> Option<String> {
        // Search from innermost to outermost scope
        for scope in self.variable_scopes.iter().rev() {
            if let Some(variable) = scope.get(name) {
                return variable.format_hint.clone();
            }
        }
        None
    }

    /// Get the format overrides for a variable (if any)
    ///
    /// Returns parameter overrides from type alias, e.g., { "decimals": 4 }
    /// for a variable declared as `let x: Percent4` where `type Percent4 = Percent { decimals: 4 }`
    pub fn get_variable_format_overrides(&self, name: &str) -> Option<HashMap<String, ValueWord>> {
        // Search from innermost to outermost scope
        for scope in self.variable_scopes.iter().rev() {
            if let Some(variable) = scope.get(name) {
                return variable.format_overrides.clone();
            }
        }
        None
    }

    /// Get both format hint and overrides for a variable
    pub fn get_variable_format_info(
        &self,
        name: &str,
    ) -> (Option<String>, Option<HashMap<String, ValueWord>>) {
        for scope in self.variable_scopes.iter().rev() {
            if let Some(variable) = scope.get(name) {
                return (
                    variable.format_hint.clone(),
                    variable.format_overrides.clone(),
                );
            }
        }
        (None, None)
    }

    /// Declare variables matching a pattern
    pub fn declare_pattern(
        &mut self,
        pattern: &shape_ast::ast::DestructurePattern,
        kind: shape_ast::ast::VarKind,
        value: ValueWord,
    ) -> Result<()> {
        use shape_ast::ast::DestructurePattern;

        match pattern {
            DestructurePattern::Identifier(name, _) => {
                self.declare_variable(name, kind, Some(value))
            }
            DestructurePattern::Array(patterns) => {
                // Destructure array
                if let Some(view) = value.as_any_array() {
                    let arr = view.to_generic();
                    let mut rest_index = None;
                    for (i, pattern) in patterns.iter().enumerate() {
                        if let DestructurePattern::Rest(inner) = pattern {
                            rest_index = Some(i);
                            let rest_values = if i <= arr.len() {
                                arr[i..].to_vec()
                            } else {
                                Vec::new()
                            };
                            self.declare_pattern(
                                inner,
                                kind,
                                ValueWord::from_array(Arc::new(rest_values)),
                            )?;
                            break;
                        } else {
                            let val = arr.get(i).map(|nb| nb.clone()).unwrap_or(ValueWord::none());
                            self.declare_pattern(pattern, kind, val)?;
                        }
                    }

                    if rest_index.is_none() && patterns.len() > arr.len() {
                        for pattern in &patterns[arr.len()..] {
                            self.declare_pattern(pattern, kind, ValueWord::none())?;
                        }
                    }
                    Ok(())
                } else {
                    Err(ShapeError::RuntimeError {
                        message: "Cannot destructure non-array value as array".to_string(),
                        location: None,
                    })
                }
            }
            DestructurePattern::Object(fields) => {
                // Destructure object
                if let Some(obj) = crate::type_schema::typed_object_to_hashmap(&value) {
                    for field in fields {
                        if field.key == "..." {
                            // Handle rest pattern in object
                            if let DestructurePattern::Rest(rest_pattern) = &field.pattern {
                                if let DestructurePattern::Identifier(rest_name, _) =
                                    rest_pattern.as_ref()
                                {
                                    // Collect remaining fields
                                    let rest_pairs: Vec<(&str, ValueWord)> = obj
                                        .iter()
                                        .filter(|(k, _)| {
                                            !fields.iter().any(|f| f.key == **k && f.key != "...")
                                        })
                                        .map(|(k, v)| (k.as_str(), v.clone()))
                                        .collect();
                                    let rest_val =
                                        crate::type_schema::typed_object_from_pairs(&rest_pairs);
                                    self.declare_variable(rest_name, kind, Some(rest_val))?;
                                }
                            }
                        } else {
                            let val = obj.get(&field.key).cloned().unwrap_or(ValueWord::none());
                            self.declare_pattern(&field.pattern, kind, val)?;
                        }
                    }
                    Ok(())
                } else {
                    Err(ShapeError::RuntimeError {
                        message: "Cannot destructure non-object value as object".to_string(),
                        location: None,
                    })
                }
            }
            DestructurePattern::Rest(_) => {
                // Rest patterns should be handled in array/object context
                Err(ShapeError::RuntimeError {
                    message: "Rest pattern cannot be used at top level".to_string(),
                    location: None,
                })
            }
            DestructurePattern::Decomposition(bindings) => {
                // Decomposition extracts component types from an intersection object
                if crate::type_schema::typed_object_to_hashmap(&value).is_some() {
                    for binding in bindings {
                        self.declare_variable(&binding.name, kind, Some(value.clone()))?;
                    }
                    Ok(())
                } else {
                    Err(ShapeError::RuntimeError {
                        message: "Cannot decompose non-object value".to_string(),
                        location: None,
                    })
                }
            }
        }
    }

    /// Set variables matching a pattern (for assignments)
    pub fn set_pattern(
        &mut self,
        pattern: &shape_ast::ast::DestructurePattern,
        value: ValueWord,
    ) -> Result<()> {
        use shape_ast::ast::DestructurePattern;

        match pattern {
            DestructurePattern::Identifier(name, _) => self.set_variable(name, value),
            DestructurePattern::Array(patterns) => {
                // Destructure array
                if let Some(view) = value.as_any_array() {
                    let arr = view.to_generic();
                    let mut rest_index = None;
                    for (i, pattern) in patterns.iter().enumerate() {
                        if let DestructurePattern::Rest(inner) = pattern {
                            rest_index = Some(i);
                            let rest_values = if i <= arr.len() {
                                arr[i..].to_vec()
                            } else {
                                Vec::new()
                            };
                            self.set_pattern(inner, ValueWord::from_array(Arc::new(rest_values)))?;
                            break;
                        } else {
                            let val = arr.get(i).map(|nb| nb.clone()).unwrap_or(ValueWord::none());
                            self.set_pattern(pattern, val)?;
                        }
                    }

                    if rest_index.is_none() && patterns.len() > arr.len() {
                        for pattern in &patterns[arr.len()..] {
                            self.set_pattern(pattern, ValueWord::none())?;
                        }
                    }
                    Ok(())
                } else {
                    Err(ShapeError::RuntimeError {
                        message: "Cannot destructure non-array value as array".to_string(),
                        location: None,
                    })
                }
            }
            DestructurePattern::Object(fields) => {
                // Destructure object
                if let Some(obj) = crate::type_schema::typed_object_to_hashmap(&value) {
                    for field in fields {
                        if field.key == "..." {
                            // Handle rest pattern in object
                            if let DestructurePattern::Rest(rest_pattern) = &field.pattern {
                                if let DestructurePattern::Identifier(rest_name, _) =
                                    rest_pattern.as_ref()
                                {
                                    // Collect remaining fields
                                    let rest_pairs: Vec<(&str, ValueWord)> = obj
                                        .iter()
                                        .filter(|(k, _)| {
                                            !fields.iter().any(|f| f.key == **k && f.key != "...")
                                        })
                                        .map(|(k, v)| (k.as_str(), v.clone()))
                                        .collect();
                                    let rest_val =
                                        crate::type_schema::typed_object_from_pairs(&rest_pairs);
                                    self.set_variable(rest_name, rest_val)?;
                                }
                            }
                        } else {
                            let val = obj.get(&field.key).cloned().unwrap_or(ValueWord::none());
                            self.set_pattern(&field.pattern, val)?;
                        }
                    }
                    Ok(())
                } else {
                    Err(ShapeError::RuntimeError {
                        message: "Cannot destructure non-object value as object".to_string(),
                        location: None,
                    })
                }
            }
            DestructurePattern::Rest(_) => {
                // Rest patterns should be handled in array/object context
                Err(ShapeError::RuntimeError {
                    message: "Rest pattern cannot be used at top level".to_string(),
                    location: None,
                })
            }
            DestructurePattern::Decomposition(bindings) => {
                // Decomposition sets variables by extracting component types
                if crate::type_schema::typed_object_to_hashmap(&value).is_some() {
                    for binding in bindings {
                        // For now, set each binding to the full object
                        // Full implementation requires TypeSchema lookup
                        self.set_variable(&binding.name, value.clone())?;
                    }
                    Ok(())
                } else {
                    Err(ShapeError::RuntimeError {
                        message: "Cannot decompose non-object value".to_string(),
                        location: None,
                    })
                }
            }
        }
    }

    /// Get all variable names currently in scope
    pub fn get_all_variable_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        // Collect names from all scopes (outer to inner)
        for scope in &self.variable_scopes {
            for name in scope.keys() {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
        }
        names
    }

    /// Get the kind of a variable (let, var, const)
    pub fn get_variable_kind(&self, name: &str) -> Option<VarKind> {
        // Search from innermost to outermost scope
        for scope in self.variable_scopes.iter().rev() {
            if let Some(variable) = scope.get(name) {
                return Some(variable.kind);
            }
        }
        None
    }

    /// Get all root-scope binding names (from the outermost scope).
    ///
    /// This is useful for REPL persistence where we need to inform the
    /// bytecode compiler about bindings from previous sessions.
    pub fn root_scope_binding_names(&self) -> Vec<String> {
        if let Some(root_scope) = self.variable_scopes.first() {
            root_scope.keys().cloned().collect()
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variable_let_creation() {
        let var = Variable::new(VarKind::Let, Some(ValueWord::from_f64(42.0)));
        assert!(var.is_initialized);
        assert!(!var.is_function_scoped);
        assert!(var.can_assign());
    }

    #[test]
    fn test_variable_const_creation() {
        let var = Variable::new(VarKind::Const, Some(ValueWord::from_f64(42.0)));
        assert!(var.is_initialized);
        assert!(!var.can_assign()); // Const cannot be reassigned
    }

    #[test]
    fn test_variable_var_creation() {
        let var = Variable::new(
            VarKind::Var,
            Some(ValueWord::from_string(std::sync::Arc::new(
                "hello".to_string(),
            ))),
        );
        assert!(var.is_initialized);
        assert!(var.is_function_scoped);
        assert!(var.can_assign());
    }

    #[test]
    fn test_variable_uninitialized() {
        let var = Variable::new(VarKind::Let, None);
        assert!(!var.is_initialized);
        assert!(var.get_value().is_err());
    }

    #[test]
    fn test_variable_assignment() {
        let mut var = Variable::new(VarKind::Let, Some(ValueWord::from_f64(1.0)));
        assert!(var.assign(ValueWord::from_f64(2.0)).is_ok());
        assert_eq!(var.get_value().unwrap().as_f64(), Some(2.0));
    }

    #[test]
    fn test_const_reassignment_fails() {
        let mut var = Variable::new(VarKind::Const, Some(ValueWord::from_f64(1.0)));
        assert!(var.assign(ValueWord::from_f64(2.0)).is_err());
    }

    #[test]
    fn test_const_initial_assignment() {
        let mut var = Variable::new(VarKind::Const, None);
        assert!(var.can_assign()); // Can assign during initialization
        assert!(var.assign(ValueWord::from_f64(42.0)).is_ok());
        assert!(!var.can_assign()); // Cannot assign after initialization
    }

    // =========================================================================
    // Format Overrides Tests
    // =========================================================================

    #[test]
    fn test_variable_with_format_overrides() {
        let mut overrides = HashMap::new();
        overrides.insert("decimals".to_string(), ValueWord::from_f64(4.0));

        let var = Variable::with_format(
            VarKind::Let,
            Some(ValueWord::from_f64(0.1234)),
            Some("Percent".to_string()),
            Some(overrides.clone()),
        );

        assert!(var.is_initialized);
        assert_eq!(var.format_hint, Some("Percent".to_string()));
        assert!(var.format_overrides.is_some());
        let stored_overrides = var.format_overrides.unwrap();
        assert_eq!(
            stored_overrides.get("decimals").and_then(|v| v.as_f64()),
            Some(4.0)
        );
    }

    #[test]
    fn test_context_declare_variable_with_format() {
        use super::super::ExecutionContext;

        let mut ctx = ExecutionContext::new_empty();
        let mut overrides = HashMap::new();
        overrides.insert("decimals".to_string(), ValueWord::from_f64(4.0));

        ctx.declare_variable_with_format(
            "rate",
            VarKind::Let,
            Some(ValueWord::from_f64(0.15)),
            Some("Percent".to_string()),
            Some(overrides),
        )
        .unwrap();

        // Verify format hint
        let hint = ctx.get_variable_format_hint("rate");
        assert_eq!(hint, Some("Percent".to_string()));

        // Verify format overrides
        let stored_overrides = ctx.get_variable_format_overrides("rate");
        assert!(stored_overrides.is_some());
        assert_eq!(
            stored_overrides
                .unwrap()
                .get("decimals")
                .and_then(|v| v.as_f64()),
            Some(4.0)
        );

        // Verify combined info
        let (hint, overrides) = ctx.get_variable_format_info("rate");
        assert_eq!(hint, Some("Percent".to_string()));
        assert!(overrides.is_some());
    }

    #[test]
    fn test_context_get_format_info_not_found() {
        use super::super::ExecutionContext;

        let ctx = ExecutionContext::new_empty();
        let (hint, overrides) = ctx.get_variable_format_info("nonexistent");
        assert!(hint.is_none());
        assert!(overrides.is_none());
    }
}
