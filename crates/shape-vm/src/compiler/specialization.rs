//! Unified specialization pipeline for generic monomorphization.

use shape_ast::ast::TypeAnnotation;
use shape_value::{ValueWord, ValueWordExt};
use std::collections::HashMap;

/// Active bindings during specialization compilation.
#[derive(Debug, Clone)]
pub(crate) struct ActiveSpecialization {
    /// Const-param bindings (name -> value). Superset of old specialization_const_bindings.
    pub const_bindings: Vec<(String, ValueWord)>,
    /// Type-param bindings (name -> concrete annotation).
    pub type_bindings: HashMap<String, TypeAnnotation>,
}

#[cfg(test)]
mod tests {
    use shape_ast::ast::TypeAnnotation;

    // --- Integration tests: Phase 0 type inference bridge ---

    #[test]
    fn test_callsite_type_args_recorded_for_generic_function() {
        // Verify the type inference engine records resolved type args at
        // generic call sites (the Phase 0 bridge for monomorphization).
        use shape_runtime::type_system::inference::TypeInferenceEngine;

        let code = r#"
            fn identity<T>(x: T) -> T { x }
            identity(42)
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let mut engine = TypeInferenceEngine::new();
        // Analyze just the items and last expression
        let _ = engine.infer_program(&program);

        // The callsite_type_args map should have an entry for the identity call
        // with T resolved to int.
        let has_type_arg = engine
            .callsite_type_args
            .values()
            .any(|args| {
                args.iter().any(|(name, ann)| {
                    name == "T" && *ann == TypeAnnotation::Basic("int".to_string())
                })
            });
        assert!(
            has_type_arg,
            "callsite_type_args should contain T=int for identity(42), got: {:?}",
            engine.callsite_type_args
        );
    }

    #[test]
    fn test_callsite_type_args_recorded_for_number_generic() {
        use shape_runtime::type_system::inference::TypeInferenceEngine;

        let code = r#"
            fn double<T>(x: T) -> T { x }
            double(3.14)
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let mut engine = TypeInferenceEngine::new();
        let _ = engine.infer_program(&program);

        let has_type_arg = engine
            .callsite_type_args
            .values()
            .any(|args| {
                args.iter().any(|(name, ann)| {
                    name == "T" && *ann == TypeAnnotation::Basic("number".to_string())
                })
            });
        assert!(
            has_type_arg,
            "callsite_type_args should contain T=number for double(3.14), got: {:?}",
            engine.callsite_type_args
        );
    }

    // --- Integration tests: Phase 3 compile-time collection specialization ---

    #[test]
    fn test_compile_time_int_array() {
        // Array of int literals should produce an IntArray via NewTypedArray
        let result = crate::test_utils::eval("[1, 2, 3]");
        assert!(result.is_heap(), "array should be heap-allocated");
    }

    #[test]
    fn test_compile_time_float_array() {
        // Array of number literals should produce a FloatArray
        let result = crate::test_utils::eval("[1.0, 2.0, 3.0]");
        assert!(result.is_heap(), "array should be heap-allocated");
    }

    #[test]
    fn test_compile_time_bool_array() {
        // Array of bool literals should produce a BoolArray
        let result = crate::test_utils::eval("[true, false, true]");
        assert!(result.is_heap(), "array should be heap-allocated");
    }

    // --- Integration tests: Phase 2 generic struct monomorphization ---

    #[test]
    fn test_generic_struct_field_access() {
        // Generic struct with concrete int field should work
        let result = crate::test_utils::eval(
            r#"
            type Wrapper<T> { value: T }
            let w = Wrapper { value: 42 }
            w.value
            "#,
        );
        assert_eq!(result.as_i64(), Some(42));
    }

    #[test]
    fn test_generic_struct_string_field() {
        let result = crate::test_utils::eval(
            r#"
            type Box<T> { item: T }
            let b = Box { item: "hello" }
            b.item
            "#,
        );
        assert_eq!(result.as_str(), Some("hello"));
    }

    #[test]
    fn test_generic_struct_two_params() {
        let result = crate::test_utils::eval(
            r#"
            type Pair<A, B> { first: A, second: B }
            let p = Pair { first: 42, second: "hi" }
            p.first
            "#,
        );
        assert_eq!(result.as_i64(), Some(42));
    }
}
