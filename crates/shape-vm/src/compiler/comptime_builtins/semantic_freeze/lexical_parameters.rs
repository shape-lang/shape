//! Frozen identities for active lexical generic-parameter scopes.

use std::collections::HashMap;

use super::{FrozenTypeIdentity, SemanticFreeze};

/// Effective name lookup plus the conservative ordered cache context.
///
/// Later scopes shadow earlier spellings for lookup. Their distinct outer
/// identities remain in `ordered_identities`: closure-inlined compilation may
/// depend on any active lexical owner even when an inner scope reuses a name.
#[derive(Debug, Clone)]
pub(super) struct LexicalParameters {
    effective_by_name: HashMap<String, FrozenTypeIdentity>,
    ordered_identities: Vec<FrozenTypeIdentity>,
}

impl LexicalParameters {
    pub(super) fn new(
        base: &SemanticFreeze,
        parameter_scopes: impl IntoIterator<Item = (String, Vec<String>)>,
    ) -> Self {
        let mut effective_by_name = HashMap::new();
        let mut ordered_identities = Vec::new();
        for (parameter_owner, type_params) in parameter_scopes {
            for name in type_params {
                // A base identity was interned first and therefore wins over a
                // same-spelled parameter, matching the canonical index.
                if base.identity_of(&name).is_some() {
                    continue;
                }
                let identity = FrozenTypeIdentity::from_canonical_descriptor(&format!(
                    "parameter:{parameter_owner}:{name}"
                ));
                if !ordered_identities.contains(&identity) {
                    ordered_identities.push(identity);
                }
                effective_by_name.insert(name, identity);
            }
        }
        Self {
            effective_by_name,
            ordered_identities,
        }
    }

    pub(super) fn identity_of(&self, name: &str) -> Option<FrozenTypeIdentity> {
        self.effective_by_name.get(name).copied()
    }

    pub(super) fn contains_name(&self, name: &str) -> bool {
        self.effective_by_name.contains_key(name)
    }

    pub(super) fn contains_identity(&self, identity: FrozenTypeIdentity) -> bool {
        self.ordered_identities.contains(&identity)
    }

    pub(super) fn ordered_identities(&self) -> &[FrozenTypeIdentity] {
        &self.ordered_identities
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::compiler::BytecodeCompiler;
    use crate::compiler::comptime_builtins::semantic_freeze::{FreezeOverlay, FrozenTypeCategory};

    #[test]
    fn lexical_overlay_retains_distinct_shadowed_parameter_identities() {
        let freeze = SemanticFreeze::freeze(&BytecodeCompiler::new()).expect("empty unit freezes");
        let overlay = FreezeOverlay::new_with_parameter_scopes(
            Arc::clone(&freeze),
            [
                ("outer".to_string(), vec!["T".to_string()]),
                ("inner".to_string(), vec!["T".to_string()]),
            ],
        );

        let identities = overlay.lexical_parameter_identities();
        assert_eq!(identities.len(), 2);
        assert_ne!(
            identities[0], identities[1],
            "different lexical owners must issue distinct Parameter identities"
        );
        assert_eq!(
            overlay.identity_of("T"),
            Some(identities[1]),
            "ordinary name lookup retains inner lexical shadowing"
        );
        assert_eq!(
            overlay.category_of(identities[0]),
            Ok(FrozenTypeCategory::Parameter),
            "the shadowed outer identity remains classifiable when carried structurally"
        );
        assert_eq!(
            overlay.category_of(identities[1]),
            Ok(FrozenTypeCategory::Parameter)
        );
    }
}
