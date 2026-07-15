//! Closed semantic specialization evidence.

use shape_ast::ast::TypeAnnotation;

use super::FrozenSemanticTypeProjection;

/// A semantic specialization argument closed under the evidence that was
/// active when the call site was prepared.
///
/// The annotation contains no inference variable that still depends on an
/// enclosing specialization frame. The projection is issued at the same
/// boundary and is the only cache-key identity for the closed annotation.
/// Construction stays private to the semantic-freeze projection boundary so
/// a raw inference candidate cannot masquerade as closed evidence.
#[derive(Debug, Clone)]
pub(crate) struct ClosedSemanticType {
    annotation: TypeAnnotation,
    projection: FrozenSemanticTypeProjection,
}

impl ClosedSemanticType {
    pub(super) fn new(
        annotation: TypeAnnotation,
        projection: FrozenSemanticTypeProjection,
    ) -> Self {
        Self {
            annotation,
            projection,
        }
    }

    pub(crate) fn annotation(&self) -> &TypeAnnotation {
        &self.annotation
    }

    pub(crate) fn projection(&self) -> &FrozenSemanticTypeProjection {
        &self.projection
    }
}
