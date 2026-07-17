//! Frozen, compilation-order-independent closure specialization identity.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};

use shape_ast::ast::CaptureMode;
use shape_runtime::comptime_reflection::FrozenTypeCategory;
use shape_value::v2::closure_layout::CaptureKind;

use super::super::{
    CallableSemanticEvidence, CallableSemanticType, CapturePack, CaptureSemanticEvidence,
    CaptureSemanticType,
};
use super::GeneratedCaptureDescriptorView;

/// Public opaque projection of one compiler-frozen semantic type.
///
/// Equality, ordering, and hashing use only the freeze-issued category and
/// 128-bit semantic identity. `presentation` is diagnostic text: it is never
/// parsed and never participates in an identity decision.
#[derive(Debug, Clone)]
pub struct GeneratedCaptureSemanticType {
    category: FrozenTypeCategory,
    identity_components: (i64, i64),
    presentation: String,
}

impl GeneratedCaptureSemanticType {
    fn from_capture_type(ty: &CaptureSemanticType) -> Self {
        Self::from_parts(ty.category(), ty.identity_components(), ty.presentation())
    }

    fn from_callable_type(ty: &CallableSemanticType) -> Self {
        Self::from_parts(ty.category(), ty.identity_components(), ty.presentation())
    }

    fn from_parts(
        category: FrozenTypeCategory,
        identity_components: (i64, i64),
        presentation: &str,
    ) -> Self {
        Self {
            category,
            identity_components,
            presentation: presentation.to_string(),
        }
    }

    fn merge_diagnostic_presentation(&mut self, other: &Self) -> Result<(), ()> {
        if self != other {
            return Err(());
        }
        if other.presentation.as_str() < self.presentation.as_str() {
            self.presentation.clone_from(&other.presentation);
        }
        Ok(())
    }

    pub fn category(&self) -> FrozenTypeCategory {
        self.category
    }

    pub fn identity_components(&self) -> (i64, i64) {
        self.identity_components
    }

    pub fn presentation(&self) -> &str {
        &self.presentation
    }

    /// Stable diagnostic/debug rendering. It is never parsed as identity.
    pub fn canonical_descriptor(&self) -> String {
        let (high, low) = self.identity_components;
        format!(
            "{}:{:016x}{:016x}",
            self.category.variant_name(),
            high as u64,
            low as u64,
        )
    }

    #[cfg(test)]
    pub(super) fn for_test(
        category: FrozenTypeCategory,
        identity_components: (i64, i64),
        presentation: &str,
    ) -> Self {
        Self::from_parts(category, identity_components, presentation)
    }
}

impl PartialEq for GeneratedCaptureSemanticType {
    fn eq(&self, other: &Self) -> bool {
        self.category == other.category && self.identity_components == other.identity_components
    }
}

impl Eq for GeneratedCaptureSemanticType {}

impl Hash for GeneratedCaptureSemanticType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.category.catalog_ordinal().hash(state);
        self.identity_components.hash(state);
    }
}

impl PartialOrd for GeneratedCaptureSemanticType {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GeneratedCaptureSemanticType {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.category.catalog_ordinal(), self.identity_components)
            .cmp(&(other.category.catalog_ordinal(), other.identity_components))
    }
}

impl fmt::Display for GeneratedCaptureSemanticType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.presentation)
    }
}

/// Canonicalize diagnostic spelling for every equal semantic identity in one
/// finished query. Runtime normally supplies one canonical spelling, but a
/// conflicting producer must not make display depend on artifact insertion
/// order. The lexicographic minimum is presentation-only and never feeds back
/// into equality, hashing, ordering, or compiler semantics.
pub(super) fn normalize_semantic_presentations(captures: &mut [GeneratedCaptureDescriptorView]) {
    let mut presentations = BTreeMap::new();
    for capture in captures.iter() {
        for specialization in &capture.specializations {
            observe_presentation(&mut presentations, &specialization.capture_type);
            for ty in &specialization.identity.capture_types {
                observe_presentation(&mut presentations, ty);
            }
            observe_presentation(&mut presentations, &specialization.identity.callable_type);
        }
    }
    for capture in captures {
        for specialization in &mut capture.specializations {
            apply_presentation(&presentations, &mut specialization.capture_type);
            for ty in &mut specialization.identity.capture_types {
                apply_presentation(&presentations, ty);
            }
            apply_presentation(&presentations, &mut specialization.identity.callable_type);
        }
    }
}

type SemanticTypeKey = (u16, (i64, i64));

fn semantic_type_key(ty: &GeneratedCaptureSemanticType) -> SemanticTypeKey {
    (ty.category.catalog_ordinal(), ty.identity_components)
}

fn observe_presentation(
    presentations: &mut BTreeMap<SemanticTypeKey, String>,
    ty: &GeneratedCaptureSemanticType,
) {
    presentations
        .entry(semantic_type_key(ty))
        .and_modify(|presentation| {
            if ty.presentation.as_str() < presentation.as_str() {
                presentation.clone_from(&ty.presentation);
            }
        })
        .or_insert_with(|| ty.presentation.clone());
}

fn apply_presentation(
    presentations: &BTreeMap<SemanticTypeKey, String>,
    ty: &mut GeneratedCaptureSemanticType,
) {
    if let Some(presentation) = presentations.get(&semantic_type_key(ty)) {
        ty.presentation.clone_from(presentation);
    }
}

/// Structural specialization identity for one compiled closure instance.
///
/// ABI carriers, compiler registries, inference tables, opaque registry IDs,
/// and `func_idx` are deliberately absent. The identity is the frozen capture
/// layout plus the frozen whole-callable semantics carried by the pack.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GeneratedCaptureSpecializationIdentity {
    capture_types: Vec<GeneratedCaptureSemanticType>,
    capture_modes: Vec<Option<CaptureMode>>,
    capture_kinds: Vec<CaptureKind>,
    callable_type: GeneratedCaptureSemanticType,
}

impl GeneratedCaptureSpecializationIdentity {
    pub fn capture_types(&self) -> &[GeneratedCaptureSemanticType] {
        &self.capture_types
    }

    pub fn capture_modes(&self) -> &[Option<CaptureMode>] {
        &self.capture_modes
    }

    pub fn capture_kinds(&self) -> &[CaptureKind] {
        &self.capture_kinds
    }

    pub fn callable_type(&self) -> &GeneratedCaptureSemanticType {
        &self.callable_type
    }

    /// Stable diagnostic/debug rendering. It is never parsed as identity.
    pub fn canonical_descriptor(&self) -> String {
        let captures = self
            .capture_types
            .iter()
            .zip(&self.capture_modes)
            .zip(&self.capture_kinds)
            .map(|((ty, mode), kind)| {
                let mode = mode.map_or("inferred", CaptureMode::variant_name);
                let kind = super::super::capture_kind_spelling(*kind);
                format!("{}:{mode}:{kind}", ty.canonical_descriptor())
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "captures:[{captures}]:callable:{}",
            self.callable_type.canonical_descriptor(),
        )
    }

    fn merge_diagnostic_presentation(&mut self, other: &Self) -> Result<(), ()> {
        if self != other {
            return Err(());
        }
        for (current, candidate) in self.capture_types.iter_mut().zip(&other.capture_types) {
            current.merge_diagnostic_presentation(candidate)?;
        }
        self.callable_type
            .merge_diagnostic_presentation(&other.callable_type)
    }

    #[cfg(test)]
    pub(super) fn for_test(
        capture_types: Vec<GeneratedCaptureSemanticType>,
        capture_modes: Vec<Option<CaptureMode>>,
        capture_kinds: Vec<CaptureKind>,
        callable_type: GeneratedCaptureSemanticType,
    ) -> Self {
        Self {
            capture_types,
            capture_modes,
            capture_kinds,
            callable_type,
        }
    }
}

/// One exact descriptor type within one structural specialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedCaptureSpecialization {
    identity: GeneratedCaptureSpecializationIdentity,
    capture_type: GeneratedCaptureSemanticType,
}

impl GeneratedCaptureSpecialization {
    pub fn identity(&self) -> &GeneratedCaptureSpecializationIdentity {
        &self.identity
    }

    pub fn capture_type(&self) -> &GeneratedCaptureSemanticType {
        &self.capture_type
    }

    pub(super) fn merge_diagnostic_presentation(&mut self, other: &Self) -> Result<(), ()> {
        if self != other {
            return Err(());
        }
        self.identity
            .merge_diagnostic_presentation(&other.identity)?;
        self.capture_type
            .merge_diagnostic_presentation(&other.capture_type)
    }

    #[cfg(test)]
    pub(super) fn for_test(
        identity: GeneratedCaptureSpecializationIdentity,
        capture_type: GeneratedCaptureSemanticType,
    ) -> Self {
        Self {
            identity,
            capture_type,
        }
    }
}

pub(super) fn specialization_for(
    pack: &CapturePack,
    descriptor_ordinal: usize,
) -> Result<GeneratedCaptureSpecialization, String> {
    let capture_types = pack
        .descriptors
        .iter()
        .enumerate()
        .map(|(ordinal, descriptor)| match &descriptor.semantic_type {
            CaptureSemanticEvidence::Exact(ty) => {
                Ok(GeneratedCaptureSemanticType::from_capture_type(ty))
            }
            CaptureSemanticEvidence::Unavailable(issue) => Err(format!(
                "capture descriptor {ordinal} semantic evidence is unavailable ({:?}): {}",
                issue.kind(),
                issue.detail(),
            )),
            CaptureSemanticEvidence::Conflict(issue) => Err(format!(
                "capture descriptor {ordinal} semantic evidence conflicts ({:?}): {}",
                issue.kind(),
                issue.detail(),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let capture_type = capture_types
        .get(descriptor_ordinal)
        .cloned()
        .ok_or_else(|| "capture descriptor ordinal is outside its closure pack".to_string())?;
    let callable_type = match pack.callable_semantic_evidence() {
        CallableSemanticEvidence::Exact(ty) => GeneratedCaptureSemanticType::from_callable_type(ty),
        CallableSemanticEvidence::Unavailable(issue) => {
            return Err(format!(
                "whole-callable semantic evidence is unavailable ({:?}): {}",
                issue.kind(),
                issue.detail(),
            ));
        }
        CallableSemanticEvidence::Conflict(issue) => {
            return Err(format!(
                "whole-callable semantic evidence conflicts ({:?}): {}",
                issue.kind(),
                issue.detail(),
            ));
        }
    };
    let identity = GeneratedCaptureSpecializationIdentity {
        capture_types,
        capture_modes: pack
            .descriptors
            .iter()
            .map(|descriptor| descriptor.declared)
            .collect(),
        capture_kinds: pack
            .descriptors
            .iter()
            .map(|descriptor| descriptor.lowered)
            .collect(),
        callable_type,
    };
    Ok(GeneratedCaptureSpecialization {
        identity,
        capture_type,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};

    use super::*;

    #[test]
    fn presentation_is_not_semantic_identity() {
        let first = GeneratedCaptureSemanticType::for_test(
            FrozenTypeCategory::Callable,
            (11, 29),
            "fn(value: int) -> int",
        );
        let renamed = GeneratedCaptureSemanticType::for_test(
            FrozenTypeCategory::Callable,
            (11, 29),
            "fn(other: int) -> int",
        );

        assert_eq!(first, renamed);
        assert_eq!(BTreeSet::from([first.clone(), renamed.clone()]).len(), 1);
        assert_eq!(HashSet::from([first.clone(), renamed]).len(), 1);
        assert!(!first.canonical_descriptor().contains("value"));
        assert_eq!(first.to_string(), "fn(value: int) -> int");
    }

    #[test]
    fn category_and_frozen_identity_both_participate() {
        let callable = GeneratedCaptureSemanticType::for_test(
            FrozenTypeCategory::Callable,
            (11, 29),
            "callable",
        );
        let different_identity = GeneratedCaptureSemanticType::for_test(
            FrozenTypeCategory::Callable,
            (11, 30),
            "callable",
        );
        let different_category = GeneratedCaptureSemanticType::for_test(
            FrozenTypeCategory::Nominal,
            (11, 29),
            "callable",
        );

        assert_ne!(callable, different_identity);
        assert_ne!(callable, different_category);
    }
}
