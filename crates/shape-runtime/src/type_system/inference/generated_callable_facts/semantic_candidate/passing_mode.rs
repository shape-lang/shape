//! Callable parameter semantics that inference's function carrier erases.

use shape_ast::ast::{FunctionParameter, TypeAnnotation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SemanticPassingMode {
    ByValue,
    SharedBorrow,
    ExclusiveBorrow,
}

impl SemanticPassingMode {
    pub(super) fn from_function_parameter(parameter: &FunctionParameter) -> Self {
        if parameter.is_mut_reference || parameter.is_out {
            Self::ExclusiveBorrow
        } else if parameter.is_reference {
            Self::SharedBorrow
        } else {
            Self::ByValue
        }
    }

    pub(super) fn from_annotation(annotation: &TypeAnnotation) -> Self {
        match annotation {
            TypeAnnotation::Borrow { mutable: true, .. } => Self::ExclusiveBorrow,
            TypeAnnotation::Borrow { mutable: false, .. } => Self::SharedBorrow,
            _ => Self::ByValue,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticCallableParameterShape {
    pub(super) optional: bool,
    pub(super) passing_mode: SemanticPassingMode,
}

impl SemanticCallableParameterShape {
    #[must_use]
    pub fn optional(&self) -> bool {
        self.optional
    }

    #[must_use]
    pub fn passing_mode(&self) -> SemanticPassingMode {
        self.passing_mode
    }
}
