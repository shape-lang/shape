//! Disjoint legacy-ABI and exact-semantic specialization storage.

use std::collections::HashMap;

use super::super::semantic_specialization::{
    LegacySpecializationKey, PreparedSemanticSpecialization, SemanticSpecializationKey,
};

/// Cache mapping typed specialization keys to compiled function indices.
///
/// Exact entries are physically separate from ABI-only entries. Diagnostic
/// iteration may project both to their ABI spelling, but no execution lookup
/// crosses between the maps.
#[derive(Debug, Default, Clone)]
pub struct MonomorphizationCache {
    entries: HashMap<LegacySpecializationKey, u16>,
    exact_entries: HashMap<SemanticSpecializationKey, u16>,
}

impl MonomorphizationCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up only the legacy ABI execution domain.
    #[cfg(test)]
    pub fn lookup(&self, mono_key: &str) -> Option<u16> {
        self.lookup_legacy(&LegacySpecializationKey::new(mono_key.to_string()))
    }

    /// Insert only into the legacy ABI execution domain.
    #[cfg(test)]
    pub fn insert(&mut self, mono_key: String, function_idx: u16) {
        self.insert_legacy(LegacySpecializationKey::new(mono_key), function_idx);
    }

    fn lookup_legacy(&self, key: &LegacySpecializationKey) -> Option<u16> {
        self.entries.get(key).copied()
    }

    fn insert_legacy(&mut self, key: LegacySpecializationKey, function_idx: u16) {
        self.entries.insert(key, function_idx);
    }

    pub(crate) fn lookup_exact(&self, key: &SemanticSpecializationKey) -> Option<u16> {
        self.exact_entries.get(key).copied()
    }

    pub(crate) fn insert_exact(&mut self, key: SemanticSpecializationKey, function_idx: u16) {
        self.exact_entries.insert(key, function_idx);
    }

    fn remove_legacy(&mut self, key: &LegacySpecializationKey) {
        self.entries.remove(key);
    }

    fn remove_exact(&mut self, key: &SemanticSpecializationKey) {
        self.exact_entries.remove(key);
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len() + self.exact_entries.len()
    }

    #[allow(dead_code)]
    pub(crate) fn legacy_len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    pub(crate) fn exact_len(&self) -> usize {
        self.exact_entries.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.exact_entries.is_empty()
    }

    /// Diagnostic view over every typed entry. Exact keys deliberately expose
    /// only their ABI prefix here; this iterator is never used for lookup.
    #[allow(dead_code)]
    pub fn iter(&self) -> impl Iterator<Item = (&String, &u16)> {
        self.entries
            .iter()
            .map(|(key, index)| (key.abi_mono_key_string(), index))
            .chain(
                self.exact_entries
                    .iter()
                    .map(|(key, index)| (key.abi_mono_key_string(), index)),
            )
    }

    /// ADR-009 C3 #14 (S1c rollback-probe fold-in, disclosed): drop every
    /// entry — BOTH domains — whose compiled function index sits at/above a
    /// truncated function-table watermark. Called from
    /// `rollback_checked_body_install`: the C2 rollback truncates
    /// `program.functions`, and a surviving cache entry above the watermark
    /// would resolve a dangling (or later reused-by-another-fn) index on the
    /// next cache hit — measured as real stale-index reuse by the S1c
    /// rollback probe (template_specialization tests).
    pub(crate) fn evict_at_or_above_function_index(&mut self, watermark: usize) {
        self.entries
            .retain(|_, index| (*index as usize) < watermark);
        self.exact_entries
            .retain(|_, index| (*index as usize) < watermark);
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.entries
            .keys()
            .map(LegacySpecializationKey::abi_mono_key_string)
            .chain(
                self.exact_entries
                    .keys()
                    .map(SemanticSpecializationKey::abi_mono_key_string),
            )
    }
}

impl PreparedSemanticSpecialization {
    pub(crate) fn cache_lookup(&self, cache: &MonomorphizationCache) -> Option<u16> {
        match self {
            Self::Legacy { key } => cache.lookup_legacy(key),
            Self::Exact { key, .. } => cache.lookup_exact(key),
        }
    }

    pub(crate) fn cache_insert(&self, cache: &mut MonomorphizationCache, function_idx: u16) {
        match self {
            Self::Legacy { key } => cache.insert_legacy(key.clone(), function_idx),
            Self::Exact { key, .. } => cache.insert_exact(key.clone(), function_idx),
        }
    }

    pub(crate) fn cache_remove(&self, cache: &mut MonomorphizationCache) {
        match self {
            Self::Legacy { key } => cache.remove_legacy(key),
            Self::Exact { key, .. } => cache.remove_exact(key),
        }
    }
}
