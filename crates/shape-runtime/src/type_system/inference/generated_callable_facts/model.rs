//! Typed structural identities and inference-fact lattice values.

use crate::type_system::Type;
use shape_ast::ast::{FunctionDef, GeneratedNodeIdentity, GeneratedNodeOrigin, GeneratedNodePath};

use super::SemanticTypeCandidate;

/// Stable structural identity of one generated AST node.
///
/// The shared AST identity contains compiler-issued declaration/structure
/// labels such as
/// `extend:Job`, `method:run`, and `closure:0`. Capture or binding source names,
/// source-map file ids/spans, owner prose, and standalone file/global traversal
/// ordinals are deliberately absent; none of those may become semantic
/// identity. A path-local `closure:N` segment is a compiler-issued structural
/// label, not a source-order key by itself.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GeneratedNodeKey(GeneratedNodeIdentity);

impl GeneratedNodeKey {
    #[must_use]
    pub fn from_origin(origin: &GeneratedNodeOrigin) -> Self {
        Self(origin.identity().clone())
    }

    #[must_use]
    pub fn expansion_fingerprint(&self) -> (i64, i64) {
        self.0.expansion().components()
    }

    /// Typed structural path retained by semantic consumers.
    #[must_use]
    pub fn path(&self) -> &GeneratedNodePath {
        self.0.node_path()
    }

    /// Rendered path components for diagnostics/query compatibility only.
    #[must_use]
    pub fn node_path(&self) -> &[String] {
        self.path().segments()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSemanticFactIssue {
    detail: String,
}

impl GeneratedSemanticFactIssue {
    pub(super) fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }

    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GeneratedCallableFact {
    Exact(SemanticTypeCandidate),
    Unavailable(GeneratedSemanticFactIssue),
    Conflict(GeneratedSemanticFactIssue),
}

impl GeneratedCallableFact {
    #[must_use]
    pub fn exact(&self) -> Option<&SemanticTypeCandidate> {
        match self {
            Self::Exact(candidate) => Some(candidate),
            Self::Unavailable(_) | Self::Conflict(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GeneratedCaptureKey {
    node: GeneratedNodeKey,
    ordinal: u16,
}

impl GeneratedCaptureKey {
    #[must_use]
    pub fn new(node: GeneratedNodeKey, ordinal: u16) -> Self {
        Self { node, ordinal }
    }

    #[must_use]
    pub fn node(&self) -> &GeneratedNodeKey {
        &self.node
    }

    #[must_use]
    pub fn ordinal(&self) -> u16 {
        self.ordinal
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GeneratedCaptureFact {
    Exact(SemanticTypeCandidate),
    Unavailable(GeneratedSemanticFactIssue),
    Conflict(GeneratedSemanticFactIssue),
}

impl GeneratedCaptureFact {
    #[must_use]
    pub fn exact(&self) -> Option<&SemanticTypeCandidate> {
        match self {
            Self::Exact(candidate) => Some(candidate),
            Self::Unavailable(_) | Self::Conflict(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum SemanticCandidateObservation {
    Candidate(SemanticTypeCandidate),
    Unavailable(GeneratedSemanticFactIssue),
    Conflict(GeneratedSemanticFactIssue),
}

impl SemanticCandidateObservation {
    pub(super) fn with_type(self, ty: Type) -> Self {
        match self {
            Self::Candidate(candidate) => candidate
                .with_resolved_type(ty)
                .map(Self::Candidate)
                .unwrap_or_else(|detail| {
                    Self::Unavailable(GeneratedSemanticFactIssue::new(detail))
                }),
            Self::Unavailable(issue) => Self::Unavailable(issue),
            Self::Conflict(issue) => Self::Conflict(issue),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct GeneratedCallableCandidate {
    pub(super) observation: SemanticCandidateObservation,
}

/// Opaque identity of one immutable declaration for one inference run.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct InferenceCallableDeclarationToken(usize);

impl InferenceCallableDeclarationToken {
    pub(super) fn of(function: &FunctionDef) -> Self {
        Self(std::ptr::from_ref(function) as usize)
    }
}

impl std::fmt::Debug for InferenceCallableDeclarationToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("InferenceCallableDeclarationToken(..)")
    }
}
