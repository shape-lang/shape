//! Typed cache, progress, and symbol identities for semantic specialization.

use shape_runtime::comptime_reflection::FrozenTypeCategory;

use super::PreparedSemanticSpecialization;
use crate::compiler::comptime_builtins::type_reflection::FrozenTypeIdentity;

/// One ordered semantic argument in an exact specialization key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct FrozenSemanticArgument {
    category: FrozenTypeCategory,
    identity: FrozenTypeIdentity,
}

impl FrozenSemanticArgument {
    pub(crate) fn new(category: FrozenTypeCategory, identity: FrozenTypeIdentity) -> Self {
        Self { category, identity }
    }
}

/// Ordered Parameter identities inherited by a lexically spliced body.
///
/// These identities are issued by the runtime-owned freeze overlay. Owner
/// names and source spellings never enter the cache authority here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub(crate) struct FrozenLexicalContext {
    ordered_parameter_identities: Vec<FrozenTypeIdentity>,
}

impl FrozenLexicalContext {
    pub(crate) fn new(ordered_parameter_identities: Vec<FrozenTypeIdentity>) -> Self {
        Self {
            ordered_parameter_identities,
        }
    }

    fn append_symbol_suffix(&self, symbol: &mut String) {
        if self.ordered_parameter_identities.is_empty() {
            return;
        }
        symbol.push_str("::lexical");
        for identity in &self.ordered_parameter_identities {
            symbol.push_str(&format!(
                "_{:016x}{:016x}",
                identity.high as u64, identity.low as u64,
            ));
        }
    }
}

/// Typed key for the ABI-only cache domain.
///
/// Most legacy entries are unscoped. Closure inlining additionally carries
/// its frozen lexical Parameter context so two owners cannot share compiled
/// reflection semantics merely because their ABI and closure shape match.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct LegacySpecializationKey {
    abi_mono_key: String,
    lexical_context: FrozenLexicalContext,
}

impl LegacySpecializationKey {
    pub(crate) fn new(abi_mono_key: String) -> Self {
        Self {
            abi_mono_key,
            lexical_context: FrozenLexicalContext::default(),
        }
    }

    pub(crate) fn with_lexical_context(mut self, lexical_context: FrozenLexicalContext) -> Self {
        self.lexical_context = lexical_context;
        self
    }

    pub(crate) fn abi_mono_key_string(&self) -> &String {
        &self.abi_mono_key
    }

    pub(crate) fn specialized_symbol(&self, mut legacy_symbol: String) -> String {
        self.lexical_context
            .append_symbol_suffix(&mut legacy_symbol);
        legacy_symbol
    }
}

/// A specialization identity that cannot collide with an ABI-only entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SemanticSpecializationKey {
    abi_mono_key: String,
    ordered_arguments: Vec<FrozenSemanticArgument>,
    lexical_context: FrozenLexicalContext,
}

impl SemanticSpecializationKey {
    pub(crate) fn new(
        abi_mono_key: String,
        ordered_arguments: Vec<FrozenSemanticArgument>,
    ) -> Self {
        Self {
            abi_mono_key,
            ordered_arguments,
            lexical_context: FrozenLexicalContext::default(),
        }
    }

    pub(crate) fn with_lexical_context(mut self, lexical_context: FrozenLexicalContext) -> Self {
        self.lexical_context = lexical_context;
        self
    }

    pub(crate) fn abi_mono_key_string(&self) -> &String {
        &self.abi_mono_key
    }

    /// Derive a linker/debug symbol from an already-issued identity.
    ///
    /// This rendering never participates in semantic identity issuance. The
    /// typed key above remains the cache authority.
    pub(crate) fn specialized_symbol(&self) -> String {
        let mut symbol = format!("{}::semantic", self.abi_mono_key);
        for argument in &self.ordered_arguments {
            symbol.push_str(&format!(
                "_{:04x}_{:016x}{:016x}",
                argument.category.catalog_ordinal(),
                argument.identity.high as u64,
                argument.identity.low as u64,
            ));
        }
        self.lexical_context.append_symbol_suffix(&mut symbol);
        symbol
    }
}

/// Cycle-detection keys preserve the same exact-vs-legacy domain boundary as
/// the cache. An ABI-only attempt cannot block or borrow an exact attempt.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SpecializationProgressKey {
    Legacy(LegacySpecializationKey),
    Exact(SemanticSpecializationKey),
}

impl From<&PreparedSemanticSpecialization> for SpecializationProgressKey {
    fn from(prepared: &PreparedSemanticSpecialization) -> Self {
        match prepared {
            PreparedSemanticSpecialization::Legacy { key } => Self::Legacy(key.clone()),
            PreparedSemanticSpecialization::Exact { key, .. } => Self::Exact(key.clone()),
        }
    }
}
