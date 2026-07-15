//! Transient lexical-binding capabilities used by generated-capture inference.

use std::collections::HashMap;

use super::TypeEnvironment;
use crate::type_system::TypeScheme;

/// Opaque inference-run identity of one live lexical binding.
///
/// Tokens are lookup capabilities. They never enter semantic type identity,
/// diagnostics, serialization, or generated-code source mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BindingToken(u64);

/// Token scopes parallel `TypeEnvironment::scopes`; this module owns their
/// lifecycle and token issuance so callers cannot accidentally split them.
#[derive(Debug, Clone)]
pub(super) struct LexicalBindingTokens {
    scopes: Vec<HashMap<String, BindingToken>>,
    next: u64,
}

impl LexicalBindingTokens {
    pub(super) fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            next: 0,
        }
    }

    fn define(&mut self, name: &str) -> BindingToken {
        let token = BindingToken(self.next);
        self.next = self
            .next
            .checked_add(1)
            .expect("TypeEnvironment binding-token overflow");
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), token);
        }
        token
    }

    fn current(&self, name: &str) -> Option<BindingToken> {
        self.scopes.last()?.get(name).copied()
    }

    fn lookup(&self, name: &str) -> Option<BindingToken> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn visible_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .scopes
            .iter()
            .flat_map(|scope| scope.keys().cloned())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    pub(super) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(super) fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }
}

impl TypeEnvironment {
    /// Define a variable in the current scope.
    pub fn define(&mut self, name: &str, scheme: TypeScheme) {
        self.define_with_token(name, scheme);
    }

    /// Define a variable and return its opaque lexical binding token.
    pub fn define_with_token(&mut self, name: &str, scheme: TypeScheme) -> BindingToken {
        let token = self.lexical_binding_tokens.define(name);
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), scheme);
        }
        token
    }

    /// Republish the scheme of the exact current binding without minting a
    /// second token. Used only for repeated passes over one AST declaration.
    pub(crate) fn redefine_with_token(
        &mut self,
        name: &str,
        expected: BindingToken,
        scheme: TypeScheme,
    ) -> Result<(), String> {
        if self.lexical_binding_tokens.current(name) != Some(expected) {
            return Err(format!(
                "binding '{name}' is no longer the expected current lexical declaration"
            ));
        }
        let Some(scope) = self.scopes.last_mut() else {
            return Err("type environment has no live lexical scope".to_string());
        };
        scope.insert(name.to_string(), scheme);
        Ok(())
    }

    /// Token of the innermost live lexical binding. Builtins have no token
    /// because they are not closure-capturable environment cells.
    pub(crate) fn lookup_binding_token(&self, name: &str) -> Option<BindingToken> {
        self.lexical_binding_tokens.lookup(name)
    }

    /// Deterministic lexical names visible at the current point. Builtins are
    /// excluded and shadowed names occur once.
    pub(crate) fn visible_binding_names(&self) -> Vec<String> {
        self.lexical_binding_tokens.visible_names()
    }

    /// Push a lexical scope and its parallel transient-capability scope.
    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.lexical_binding_tokens.push_scope();
    }

    /// Number of live scopes (1 = module scope).
    pub fn scope_depth(&self) -> usize {
        self.scopes.len()
    }

    /// Depth (1-based, innermost wins) of the scope that declares `name`.
    pub fn declaring_scope_depth(&self, name: &str) -> Option<usize> {
        self.scopes
            .iter()
            .rposition(|scope| scope.contains_key(name))
            .map(|index| index + 1)
    }

    /// Pop a lexical scope and its parallel transient-capability scope.
    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
            self.lexical_binding_tokens.pop_scope();
        }
    }
}
