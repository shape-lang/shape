//! ADR-009 D2 / C1 — node-borne generated-code provenance.
//!
//! Generated closures carry a compiler-issued semantic identity: the owning
//! expansion fingerprint plus a structural path inside that expansion. Source
//! anchors and owner names remain presentation data for diagnostics. They must
//! never change equality, hashing, or cache identity.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::span::Span;

/// The 128-bit content fingerprint of the expansion that issued a node.
///
/// This is a typed carrier, not the authority to trust a node. Trust remains
/// compilation-scoped and is checked by [`GeneratedNodeIssuer::recognizes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GeneratedExpansionFingerprint {
    expansion_high: i64,
    expansion_low: i64,
}

impl GeneratedExpansionFingerprint {
    #[must_use]
    pub fn from_components(high: i64, low: i64) -> Self {
        Self {
            expansion_high: high,
            expansion_low: low,
        }
    }

    #[must_use]
    pub fn components(self) -> (i64, i64) {
        (self.expansion_high, self.expansion_low)
    }
}

/// One opaque structural component of a generated-node path.
///
/// The compiler may extend its segment vocabulary without changing this type.
/// Validation only enforces the path encoding contract: a component is
/// non-empty, contains no control characters, and cannot contain `/` (the
/// diagnostic rendering separator). Thus malformed display fragments cannot
/// silently become semantic path components.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GeneratedNodePathSegment(String);

impl GeneratedNodePathSegment {
    pub fn new(segment: impl Into<String>) -> Result<Self, InvalidGeneratedNodePathSegment> {
        let segment = segment.into();
        if segment.is_empty() {
            return Err(InvalidGeneratedNodePathSegment::new(
                segment,
                "a structural segment cannot be empty",
            ));
        }
        if segment.contains('/') {
            return Err(InvalidGeneratedNodePathSegment::new(
                segment,
                "a structural segment cannot contain the path separator '/'",
            ));
        }
        if segment.chars().any(char::is_control) {
            return Err(InvalidGeneratedNodePathSegment::new(
                segment,
                "a structural segment cannot contain control characters",
            ));
        }
        Ok(Self(segment))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GeneratedNodePathSegment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for GeneratedNodePathSegment {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for GeneratedNodePathSegment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let segment = String::deserialize(deserializer)?;
        Self::new(segment).map_err(serde::de::Error::custom)
    }
}

/// Why a rendered path component could not become structural identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidGeneratedNodePathSegment {
    segment: String,
    reason: &'static str,
}

impl InvalidGeneratedNodePathSegment {
    fn new(segment: String, reason: &'static str) -> Self {
        Self { segment, reason }
    }
}

impl fmt::Display for InvalidGeneratedNodePathSegment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid generated-node path segment {:?}: {}",
            self.segment, self.reason
        )
    }
}

impl std::error::Error for InvalidGeneratedNodePathSegment {}

/// Typed structural path from a generated declaration to one generated node.
///
/// Empty paths remain representable because the carrier historically allowed
/// them; the compiler's declaration mint decides where a root is required.
/// The derived string slice is presentation compatibility only. Equality,
/// ordering, and hashing use the opaque structural segments.
#[derive(Debug, Clone)]
pub struct GeneratedNodePath {
    segments: Vec<GeneratedNodePathSegment>,
    rendered_segments: Vec<String>,
}

impl GeneratedNodePath {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            segments: Vec::new(),
            rendered_segments: Vec::new(),
        }
    }

    pub fn try_from_rendered_segments(
        segments: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, InvalidGeneratedNodePathSegment> {
        let segments = segments
            .into_iter()
            .map(|segment| GeneratedNodePathSegment::new(segment.into()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::from_typed_segments(segments))
    }

    /// Start at a compiler-issued declaration root.
    ///
    /// Compiler callers supply checked identifier/type renderings. An invalid
    /// renderer is an internal invariant violation and is rejected before any
    /// origin can be issued.
    #[must_use]
    pub fn decl_root(segment: impl Into<String>) -> Self {
        Self::try_from_rendered_segments([segment]).unwrap_or_else(|error| {
            panic!("compiler produced an invalid generated declaration path: {error}")
        })
    }

    /// Extend the path with one compiler-issued structural component.
    #[must_use]
    pub fn child(&self, segment: impl Into<String>) -> Self {
        self.try_child(segment).unwrap_or_else(|error| {
            panic!("compiler produced an invalid generated child path: {error}")
        })
    }

    pub fn try_child(
        &self,
        segment: impl Into<String>,
    ) -> Result<Self, InvalidGeneratedNodePathSegment> {
        let mut segments = self.segments.clone();
        segments.push(GeneratedNodePathSegment::new(segment)?);
        Ok(Self::from_typed_segments(segments))
    }

    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        (!self.segments.is_empty())
            .then(|| Self::from_typed_segments(self.segments[..self.segments.len() - 1].to_vec()))
    }

    #[must_use]
    pub fn starts_with(&self, prefix: &Self) -> bool {
        self.segments.starts_with(&prefix.segments)
    }

    /// Opaque structural components used by semantic consumers.
    #[must_use]
    pub fn typed_segments(&self) -> &[GeneratedNodePathSegment] {
        &self.segments
    }

    /// Rendered components for diagnostics and compatibility consumers only.
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.rendered_segments
    }

    #[must_use]
    pub fn render(&self) -> String {
        self.rendered_segments.join("/")
    }

    fn from_typed_segments(segments: Vec<GeneratedNodePathSegment>) -> Self {
        let rendered_segments = segments
            .iter()
            .map(|segment| segment.as_str().to_string())
            .collect();
        Self {
            segments,
            rendered_segments,
        }
    }
}

impl PartialEq for GeneratedNodePath {
    fn eq(&self, other: &Self) -> bool {
        self.segments == other.segments
    }
}

impl Eq for GeneratedNodePath {}

impl PartialOrd for GeneratedNodePath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GeneratedNodePath {
    fn cmp(&self, other: &Self) -> Ordering {
        self.segments.cmp(&other.segments)
    }
}

impl Hash for GeneratedNodePath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.segments.hash(state);
    }
}

impl Serialize for GeneratedNodePath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.segments.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for GeneratedNodePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let segments = Vec::<GeneratedNodePathSegment>::deserialize(deserializer)?;
        Ok(Self::from_typed_segments(segments))
    }
}

/// Complete semantic identity of one generated AST node.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GeneratedNodeIdentity {
    #[serde(flatten)]
    expansion: GeneratedExpansionFingerprint,
    node_path: GeneratedNodePath,
}

impl GeneratedNodeIdentity {
    fn new(expansion: GeneratedExpansionFingerprint, node_path: GeneratedNodePath) -> Self {
        Self {
            expansion,
            node_path,
        }
    }

    #[must_use]
    pub fn expansion(&self) -> GeneratedExpansionFingerprint {
        self.expansion
    }

    #[must_use]
    pub fn node_path(&self) -> &GeneratedNodePath {
        &self.node_path
    }
}

/// Provenance stamped on a generated AST node (today: `Expr::FunctionExpr`).
///
/// Semantic equality and hashing are exactly [`GeneratedNodeIdentity`]. The
/// source anchor and owner display below are intentionally excluded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedNodeOrigin {
    #[serde(flatten)]
    identity: GeneratedNodeIdentity,
    anchor_file_id: u16,
    anchor_span: Span,
    owner_display: String,
    /// Compilation-instance authority. Serialized provenance is diagnostic
    /// data, never evidence that the current compiler issued the node.
    #[serde(skip)]
    authority: Option<Arc<()>>,
}

/// Compilation-scoped capability that issues and verifies generated origins.
#[derive(Debug, Clone)]
pub struct GeneratedNodeIssuer {
    authority: Arc<()>,
}

impl Default for GeneratedNodeIssuer {
    fn default() -> Self {
        Self::new()
    }
}

impl GeneratedNodeIssuer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            authority: Arc::new(()),
        }
    }

    /// Issue a node stamp under this exact compilation capability.
    #[must_use]
    pub fn issue(
        &self,
        expansion: GeneratedExpansionFingerprint,
        node_path: GeneratedNodePath,
        anchor_file_id: u16,
        anchor_span: Span,
        owner_display: String,
    ) -> GeneratedNodeOrigin {
        GeneratedNodeOrigin {
            identity: GeneratedNodeIdentity::new(expansion, node_path),
            anchor_file_id,
            anchor_span,
            owner_display,
            authority: Some(Arc::clone(&self.authority)),
        }
    }

    #[must_use]
    pub fn recognizes(&self, origin: &GeneratedNodeOrigin) -> bool {
        origin
            .authority
            .as_ref()
            .is_some_and(|authority| Arc::ptr_eq(authority, &self.authority))
    }
}

impl PartialEq for GeneratedNodeOrigin {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl Eq for GeneratedNodeOrigin {}

impl Hash for GeneratedNodeOrigin {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity.hash(state);
    }
}

impl GeneratedNodeOrigin {
    /// Descend to one generated child while retaining expansion and diagnostic
    /// provenance.
    #[must_use]
    pub fn child(&self, segment: impl Into<String>) -> Self {
        Self {
            identity: GeneratedNodeIdentity::new(
                self.identity.expansion(),
                self.identity.node_path().child(segment),
            ),
            ..self.clone()
        }
    }

    #[must_use]
    pub fn identity(&self) -> &GeneratedNodeIdentity {
        &self.identity
    }

    /// Compatibility projection for presentation/query consumers. Semantic
    /// consumers should retain [`Self::identity`] or [`Self::path`].
    #[must_use]
    pub fn expansion_fingerprint(&self) -> (i64, i64) {
        self.identity.expansion().components()
    }

    #[must_use]
    pub fn path(&self) -> &GeneratedNodePath {
        self.identity.node_path()
    }

    /// Rendered path components for diagnostics and source mapping only.
    #[must_use]
    pub fn node_path(&self) -> &[String] {
        self.path().segments()
    }

    #[must_use]
    pub fn anchor(&self) -> (u16, Span) {
        (self.anchor_file_id, self.anchor_span)
    }

    #[must_use]
    pub fn owner_display(&self) -> &str {
        &self.owner_display
    }

    #[must_use]
    pub fn render_path(&self) -> String {
        self.path().render()
    }
}

#[cfg(test)]
mod tests;
