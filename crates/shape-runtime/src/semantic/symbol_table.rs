//! Symbol table for tracking identifiers and their types

use serde::{Deserialize, Serialize};
use shape_ast::ast::Span;
use shape_ast::ast::TypeAnnotation;
use shape_ast::error::{Result, ShapeError, span_to_location};
use std::collections::HashMap;

use super::types::Type;
use crate::type_system::environment::TypeAliasEntry;
use shape_ast::ast::{EnumDef, VarKind};

/// A symbol in the symbol table
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Symbol {
    /// Variable symbol
    Variable {
        ty: Type,
        kind: VarKind,
        is_initialized: bool,
    },
    /// Function symbol
    Function {
        params: Vec<Type>,
        returns: Type,
        defaults: Vec<bool>,
    },
    /// Module symbol (extension namespace like `duckdb`, `csv`)
    Module {
        exports: Vec<String>,
        type_annotation: TypeAnnotation,
    },
}

/// Symbol table for managing scopes and symbols
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolTable {
    /// Stack of scopes (innermost last)
    scopes: Vec<Scope>,
    /// Enum definitions
    enums: HashMap<String, EnumDef>,
    /// Type aliases with optional meta parameter overrides
    type_aliases: HashMap<String, TypeAliasEntry>,
    /// Source code for error location reporting
    source: Option<String>,
    /// Allow variable redefinition in current scope (REPL mode)
    allow_redefinition: bool,
}

/// A single scope in the symbol table
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Scope {
    /// Symbols defined in this scope
    symbols: HashMap<String, Symbol>,
}

impl SymbolTable {
    /// Create a new symbol table with root scope
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope::new()],
            enums: HashMap::new(),
            type_aliases: HashMap::new(),
            source: None,
            allow_redefinition: false,
        }
    }

    /// Set the source code for error location reporting
    pub fn set_source(&mut self, source: String) {
        self.source = Some(source);
    }

    /// Allow variable redefinition in current scope (for REPL mode)
    pub fn set_allow_redefinition(&mut self, allow: bool) {
        self.allow_redefinition = allow;
    }

    /// Create an error with location information from a span
    fn error_at(&self, span: Span, message: impl Into<String>) -> ShapeError {
        let location = self
            .source
            .as_ref()
            .map(|src| span_to_location(src, span, None));
        ShapeError::SemanticError {
            message: message.into(),
            location,
        }
    }

    /// Push a new scope
    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    /// Pop the current scope
    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    /// Define a variable in the current scope
    pub fn define_variable(
        &mut self,
        name: &str,
        ty: Type,
        kind: VarKind,
        is_initialized: bool,
    ) -> Result<()> {
        self.define_variable_at(name, ty, kind, is_initialized, Span::DUMMY)
    }

    /// Define a variable in the current scope with span for error reporting
    pub fn define_variable_at(
        &mut self,
        name: &str,
        ty: Type,
        kind: VarKind,
        is_initialized: bool,
        span: Span,
    ) -> Result<()> {
        let scope = self.scopes.last_mut().unwrap();

        if scope.symbols.contains_key(name) {
            if self.allow_redefinition {
                scope.symbols.insert(
                    name.to_string(),
                    Symbol::Variable {
                        ty,
                        kind,
                        is_initialized,
                    },
                );
                return Ok(());
            }
            return Err(self.error_at(
                span,
                format!("Variable '{}' is already defined in this scope", name),
            ));
        }

        scope.symbols.insert(
            name.to_string(),
            Symbol::Variable {
                ty,
                kind,
                is_initialized,
            },
        );
        Ok(())
    }

    /// Define a function in the current scope
    pub fn define_function(&mut self, name: &str, params: Vec<Type>, returns: Type) -> Result<()> {
        let defaults = vec![false; params.len()];
        self.define_function_at_with_defaults(name, params, returns, defaults, Span::DUMMY)
    }

    /// Define a function in the current scope with default-parameter metadata.
    pub fn define_function_with_defaults(
        &mut self,
        name: &str,
        params: Vec<Type>,
        returns: Type,
        defaults: Vec<bool>,
    ) -> Result<()> {
        self.define_function_at_with_defaults(name, params, returns, defaults, Span::DUMMY)
    }

    /// Define a function in the current scope with span for error reporting
    pub fn define_function_at(
        &mut self,
        name: &str,
        params: Vec<Type>,
        returns: Type,
        span: Span,
    ) -> Result<()> {
        let defaults = vec![false; params.len()];
        self.define_function_at_with_defaults(name, params, returns, defaults, span)
    }

    fn define_function_at_with_defaults(
        &mut self,
        name: &str,
        params: Vec<Type>,
        returns: Type,
        defaults: Vec<bool>,
        span: Span,
    ) -> Result<()> {
        let scope = self.scopes.last_mut().unwrap();

        if scope.symbols.contains_key(name) {
            return Err(self.error_at(
                span,
                format!("Function '{}' is already defined in this scope", name),
            ));
        }

        scope.symbols.insert(
            name.to_string(),
            Symbol::Function {
                params,
                returns,
                defaults,
            },
        );
        Ok(())
    }

    /// Define an enum (always root-scoped)
    pub fn define_enum(&mut self, enum_def: EnumDef) -> Result<()> {
        if self.enums.contains_key(&enum_def.name) {
            return Err(self.error_at(
                Span::DUMMY,
                format!("Enum '{}' is already defined", enum_def.name),
            ));
        }

        self.enums.insert(enum_def.name.clone(), enum_def);
        Ok(())
    }

    /// Look up an enum by name
    pub fn lookup_enum(&self, name: &str) -> Option<&EnumDef> {
        self.enums.get(name)
    }

    /// Define a type alias (e.g., `type Candle = { ... }`)
    pub fn define_type_alias(&mut self, name: &str, type_annotation: TypeAnnotation) -> Result<()> {
        self.define_type_alias_at(name, type_annotation, None, Span::DUMMY)
    }

    /// Define a type alias with span for error reporting
    pub fn define_type_alias_at(
        &mut self,
        name: &str,
        type_annotation: TypeAnnotation,
        meta_param_overrides: Option<HashMap<String, shape_ast::ast::Expr>>,
        span: Span,
    ) -> Result<()> {
        if self.type_aliases.contains_key(name) {
            return Err(self.error_at(span, format!("Type '{}' is already defined", name)));
        }

        self.type_aliases.insert(
            name.to_string(),
            TypeAliasEntry {
                type_annotation,
                meta_param_overrides,
            },
        );
        Ok(())
    }

    /// Look up a type alias by name
    pub fn lookup_type_alias(&self, name: &str) -> Option<&TypeAliasEntry> {
        self.type_aliases.get(name)
    }

    /// Look up a symbol in all scopes (innermost first)
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        // Search from innermost to outermost scope
        for scope in self.scopes.iter().rev() {
            if let Some(symbol) = scope.symbols.get(name) {
                return Some(symbol);
            }
        }

        None
    }

    /// Update an existing variable (for assignments)
    pub fn update_variable(&mut self, name: &str, new_ty: Type) -> Result<()> {
        self.update_variable_at(name, new_ty, Span::DUMMY)
    }

    /// Update an existing variable (for assignments) with span for error reporting
    pub fn update_variable_at(&mut self, name: &str, new_ty: Type, span: Span) -> Result<()> {
        // Find which scope contains the variable
        for scope in self.scopes.iter_mut().rev() {
            if let Some(symbol) = scope.symbols.get_mut(name) {
                match symbol {
                    Symbol::Variable {
                        ty,
                        kind,
                        is_initialized,
                    } => {
                        // Check if const and already initialized
                        if matches!(kind, VarKind::Const) && *is_initialized {
                            return Err(self.error_at(
                                span,
                                format!("Cannot reassign const variable '{}'", name),
                            ));
                        }

                        // Update the type and mark as initialized
                        *ty = new_ty;
                        *is_initialized = true;
                        return Ok(());
                    }
                    _ => return Err(self.error_at(span, format!("'{}' is not a variable", name))),
                }
            }
        }

        Err(self.error_at(span, format!("Undefined variable: '{}'", name)))
    }

    /// Look up a variable specifically
    pub fn lookup_variable(&self, name: &str) -> Option<(&Type, &VarKind, bool)> {
        match self.lookup(name)? {
            Symbol::Variable {
                ty,
                kind,
                is_initialized,
            } => Some((ty, kind, *is_initialized)),
            _ => None,
        }
    }

    /// Look up a function specifically
    pub fn lookup_function(&self, name: &str) -> Option<(&Vec<Type>, &Type, &Vec<bool>)> {
        match self.lookup(name)? {
            Symbol::Function {
                params,
                returns,
                defaults,
            } => Some((params, returns, defaults)),
            _ => None,
        }
    }

    /// Define a module in the root (first) scope. Idempotent — returns Ok if already defined.
    pub fn define_module(
        &mut self,
        name: &str,
        exports: Vec<String>,
        type_annotation: TypeAnnotation,
    ) -> Result<()> {
        let root_scope = &mut self.scopes[0];
        if root_scope.symbols.contains_key(name) {
            return Ok(()); // idempotent
        }
        root_scope.symbols.insert(
            name.to_string(),
            Symbol::Module {
                exports,
                type_annotation,
            },
        );
        Ok(())
    }

    /// Look up a module specifically
    pub fn lookup_module(&self, name: &str) -> Option<(&Vec<String>, &TypeAnnotation)> {
        match self.lookup(name)? {
            Symbol::Module {
                exports,
                type_annotation,
            } => Some((exports, type_annotation)),
            _ => None,
        }
    }

    /// Check if a name is defined in the current scope only
    pub fn is_defined_in_current_scope(&self, name: &str) -> bool {
        self.scopes.last().unwrap().symbols.contains_key(name)
    }

    /// Iterate over all symbols in all scopes
    ///
    /// Returns an iterator of (name, symbol) pairs from all scopes,
    /// with inner scopes appearing after outer scopes.
    pub fn iter_all_symbols(&self) -> impl Iterator<Item = (&str, &Symbol)> {
        self.scopes
            .iter()
            .flat_map(|scope| scope.symbols.iter().map(|(k, v)| (k.as_str(), v)))
    }

    /// Iterate over all type aliases
    pub fn iter_type_aliases(&self) -> impl Iterator<Item = (&str, &TypeAliasEntry)> {
        self.type_aliases.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Iterate over all enums
    pub fn iter_enums(&self) -> impl Iterator<Item = (&str, &EnumDef)> {
        self.enums.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl Scope {
    /// Create a new empty scope
    fn new() -> Self {
        Self {
            symbols: HashMap::new(),
        }
    }
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}
