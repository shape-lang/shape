//! Typed specialization failure boundary.

use shape_ast::error::ShapeError;

/// Resolution failures may fall back; specialized-body failures must surface.
#[derive(Debug)]
pub enum SpecializationFailure {
    Soft(ShapeError),
    Hard(ShapeError),
}

impl SpecializationFailure {
    pub fn into_error(self) -> ShapeError {
        match self {
            Self::Soft(error) | Self::Hard(error) => error,
        }
    }
}

/// Bare errors raised by specialization bookkeeping are soft. Body compile
/// sites and exact semantic quarantine opt into `Hard` explicitly.
impl From<ShapeError> for SpecializationFailure {
    fn from(error: ShapeError) -> Self {
        Self::Soft(error)
    }
}

impl From<SpecializationFailure> for ShapeError {
    fn from(failure: SpecializationFailure) -> Self {
        failure.into_error()
    }
}
