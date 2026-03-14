//! Shape Type System
//!
//! This module implements type inference, type checking, and type unification
//! for the Shape language, providing static type safety while maintaining
//! ergonomic syntax through type inference.
//!
//! ## Architecture: Storage vs Semantic Types
//!
//! - **SemanticType**: What users see in code (`Option<f64>`, `Result<Table<Row>>`)
//! - **StorageType**: How data is physically stored (NaN sentinels, bitmaps)
//!
//! This separation allows the type checker to be strict while storage can be
//! optimized for JIT/SIMD performance.
//!
//! ## Module Structure
//!
//! - `types/`: Core type definitions (Type, TypeVar, TypeScheme, TypeConstraint)
//! - `unification/`: Type unification algorithm (Robinson's algorithm)
//! - `solver/`: Constraint solving
//! - `checking/`: Pattern checking and type checking
//! - `environment/`: Type environments and scopes
//! - `inference/`: Type inference engine

pub mod checker;
pub mod checking;
pub mod constraints;
pub mod environment;
pub mod error_bridge;
pub mod errors;
pub mod exhaustiveness;
pub mod inference;
pub mod semantic;
pub mod storage;
pub mod suggestions;
pub mod typed_value;
pub mod types;
pub mod unification;
pub mod universal_error;

// Re-export from types module
pub use types::{
    BuiltinTypes, Type, TypeConstraint, TypeScheme, TypeVar, annotation_to_semantic,
    annotation_to_string, semantic_to_annotation, substitute,
};

// Re-export other public types
pub use checker::{
    TypeAnalysisMode, TypeCheckResult, TypeChecker, TypeWarning, analyze_program,
    analyze_program_with_mode,
};
pub use environment::TypeEnvironment;
pub use errors::{TypeError, TypeErrorWithLocation, TypeResult};
pub use inference::{PropertyAssignment, PropertyAssignmentCollector, TypeInferenceEngine};
pub use semantic::{EnumVariant, FunctionParam, FunctionSignature, SemanticType, TypeVarId};
pub use storage::StorageType;
pub use typed_value::TypedValue;
pub use universal_error::{ErrorDetails, ErrorLocation, UniversalError};

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::TypeAnnotation;

    #[test]
    fn test_type_to_semantic_primitives() {
        let num = BuiltinTypes::number();
        let semantic = num.to_semantic().unwrap();
        assert_eq!(semantic, SemanticType::Number);

        let string = BuiltinTypes::string();
        let semantic = string.to_semantic().unwrap();
        assert_eq!(semantic, SemanticType::String);

        let boolean = BuiltinTypes::boolean();
        let semantic = boolean.to_semantic().unwrap();
        assert_eq!(semantic, SemanticType::Bool);
    }

    #[test]
    fn test_type_to_semantic_option() {
        let option_num = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Option".into(),
            ))),
            args: vec![BuiltinTypes::number()],
        };
        let semantic = option_num.to_semantic().unwrap();
        assert_eq!(
            semantic,
            SemanticType::Option(Box::new(SemanticType::Number))
        );
    }

    #[test]
    fn test_type_to_semantic_result() {
        let result_num = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Result".into(),
            ))),
            args: vec![BuiltinTypes::number()],
        };
        let semantic = result_num.to_semantic().unwrap();
        assert_eq!(
            semantic,
            SemanticType::Result {
                ok_type: Box::new(SemanticType::Number),
                err_type: None
            }
        );
    }

    #[test]
    fn test_type_to_semantic_generic_table() {
        let table_num = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Table".into(),
            ))),
            args: vec![BuiltinTypes::number()],
        };
        let semantic = table_num.to_semantic().unwrap();
        assert_eq!(
            semantic,
            SemanticType::Generic {
                name: "Table".to_string(),
                args: vec![SemanticType::Number],
            }
        );
    }

    #[test]
    fn test_semantic_to_inference_roundtrip() {
        let original = SemanticType::Option(Box::new(SemanticType::Number));
        let inference = original.to_inference_type();
        let roundtrip = inference.to_semantic().unwrap();
        assert_eq!(original, roundtrip);
    }

    #[test]
    fn test_semantic_result_to_inference() {
        let result_type = SemanticType::Result {
            ok_type: Box::new(SemanticType::String),
            err_type: None,
        };
        let inference = result_type.to_inference_type();
        let roundtrip = inference.to_semantic().unwrap();
        assert_eq!(result_type, roundtrip);
    }
}
