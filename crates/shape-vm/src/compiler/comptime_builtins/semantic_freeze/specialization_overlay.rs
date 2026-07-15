//! Scoped generic-parameter capability for one specialized-body compile.

use std::collections::{HashMap, HashSet};

use shape_runtime::type_system::TypeVar;

use super::ClosedSemanticType;

/// Declaration-only overlays preserve stable Parameter reflection for legacy
/// ABI execution. Only exact overlays authorize projection from a declared
/// inference token to a closed semantic argument.
#[derive(Debug, Clone)]
pub(crate) struct SpecializationTypeOverlay {
    parameter_scopes: Vec<LexicalParameterScope>,
    exact_frame: bool,
    exact_arguments: HashMap<TypeVar, ClosedSemanticType>,
}

#[derive(Debug, Clone)]
struct LexicalParameterScope {
    owner: String,
    declared_names: Vec<String>,
}

impl SpecializationTypeOverlay {
    pub(crate) fn declaration_only(
        parameter_owner: impl Into<String>,
        declared_names: Vec<String>,
    ) -> Self {
        Self {
            parameter_scopes: vec![LexicalParameterScope {
                owner: parameter_owner.into(),
                declared_names,
            }],
            exact_frame: false,
            exact_arguments: HashMap::new(),
        }
    }

    pub(crate) fn exact(
        parameter_owner: impl Into<String>,
        declared_names: Vec<String>,
        arguments: impl IntoIterator<Item = (TypeVar, ClosedSemanticType)>,
    ) -> Result<Self, String> {
        let parameter_owner = parameter_owner.into();
        let mut declared_set = HashSet::new();
        for name in &declared_names {
            if !declared_set.insert(name.clone()) {
                return Err(format!(
                    "semantic specialization declares type parameter '{name}' more than once"
                ));
            }
        }
        let mut exact_arguments = HashMap::new();
        let mut observed_names = HashSet::new();
        for (declared, candidate) in arguments {
            let provenance = declared.declared_provenance().ok_or_else(|| {
                "semantic specialization argument is keyed by a non-declared inference token"
                    .to_string()
            })?;
            if !declared_set.contains(provenance.source_name()) {
                return Err(format!(
                    "semantic specialization argument '{}' is absent from the declared parameter list",
                    provenance.source_name()
                ));
            }
            if !observed_names.insert(provenance.source_name().to_string()) {
                return Err(format!(
                    "semantic specialization supplies parameter '{}' more than once",
                    provenance.source_name()
                ));
            }
            if exact_arguments.insert(declared, candidate).is_some() {
                return Err(
                    "semantic specialization supplies the same declared token more than once"
                        .to_string(),
                );
            }
        }
        if observed_names != declared_set {
            return Err(
                "semantic specialization does not supply every declared type parameter".to_string(),
            );
        }
        Ok(Self {
            parameter_scopes: vec![LexicalParameterScope {
                owner: parameter_owner,
                declared_names,
            }],
            exact_frame: true,
            exact_arguments,
        })
    }

    pub(crate) fn has_exact_arguments(&self) -> bool {
        self.exact_frame
    }

    /// Whether caller-owned syntax and a prospective inline callee use the
    /// same generic-parameter spelling.
    ///
    /// Spelling is only refusal evidence here, never semantic identity. The
    /// unannotated AST cannot say whether `T` in a spliced closure belongs to
    /// the caller or callee once both are in scope, so closure inlining must
    /// stop before cache lookup instead of selecting either name binding.
    pub(crate) fn has_lexical_parameter_name_collision(
        &self,
        inner_declared_names: &[String],
    ) -> bool {
        self.parameter_scopes.iter().any(|outer| {
            outer
                .declared_names
                .iter()
                .any(|name| inner_declared_names.contains(name))
        })
    }

    /// Compose the typed context only when caller-owned syntax is lexically
    /// spliced into a callee specialization.
    ///
    /// Ordinary recursive compilation is not lexical nesting and must install
    /// this overlay unchanged.
    ///
    /// Lexical Parameter scopes are always inherited, with the current scope
    /// last so its names shadow outer names. Closed semantic arguments cross
    /// only an uninterrupted exact-to-exact edge. A declaration-only frame is
    /// therefore a hard evidence barrier, while still preserving authored
    /// outer `type_ref(T)` expressions in inlined/generated lexical bodies.
    /// On an opaque-token collision the current frame wins.
    pub(crate) fn inherit_for_lexical_inline(&mut self, outer: &Self) {
        let current_scope = std::mem::take(&mut self.parameter_scopes);
        self.parameter_scopes = outer.parameter_scopes.clone();
        self.parameter_scopes.extend(current_scope);

        if self.exact_frame && outer.exact_frame {
            for (declared, closed) in &outer.exact_arguments {
                self.exact_arguments
                    .entry(declared.clone())
                    .or_insert_with(|| closed.clone());
            }
        }
    }

    pub(super) fn parameter_owner(&self) -> &str {
        &self
            .parameter_scopes
            .last()
            .expect("specialization overlay always has a current parameter scope")
            .owner
    }

    pub(super) fn declared_names(&self) -> &[String] {
        &self
            .parameter_scopes
            .last()
            .expect("specialization overlay always has a current parameter scope")
            .declared_names
    }

    pub(super) fn parameter_scopes(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.parameter_scopes
            .iter()
            .map(|scope| (scope.owner.as_str(), scope.declared_names.as_slice()))
    }

    pub(super) fn exact_arguments(&self) -> &HashMap<TypeVar, ClosedSemanticType> {
        &self.exact_arguments
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_name_collision_is_a_refusal_signal_not_an_identity() {
        let outer = SpecializationTypeOverlay::declaration_only("outer", vec!["T".to_string()]);

        assert!(outer.has_lexical_parameter_name_collision(&["T".to_string()]));
        assert!(!outer.has_lexical_parameter_name_collision(&["U".to_string()]));
    }
}
