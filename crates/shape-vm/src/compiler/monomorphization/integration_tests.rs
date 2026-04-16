//!
//! These tests exercise the full monomorphization pipeline: compile generic
//! stdlib calls (`map`, `filter`, `reduce`, …) for concrete element types and
//! verify that:
//!   1. The specialization cache contains the expected `mono_key` entries
//!      (via `BytecodeProgram::monomorphization_keys`).
//!   2. Two call sites with identical type args share one specialization.
//!   3. Two call sites with different type args produce two specializations.
//!   4. Concrete (non-generic) functions are NOT cached.
//!   5. The runtime result is correct end-to-end.
//!
//! ## Ignored tests
//!
//! - `test_user_defined_generic_function`: regular (non-method) function calls
//!   do not yet trigger monomorphization.

use shape_value::ValueWordExt;
// ---------------------------------------------------------------------------
// Meta-test: confirm the monomorphization module is reachable.
// ---------------------------------------------------------------------------

/// This test MUST always compile and pass — even when every other test in this
/// file is gated behind `#[cfg(any())]`. It is the canary that proves the
/// `crate::compiler::monomorphization` module path resolves and that the
/// stable parts of the cache API (Agent 3) are wired into the compiler.
#[test]
fn test_monomorphization_module_exists() {
    use crate::compiler::monomorphization::cache::{MonomorphizationCache, build_mono_key};
    use shape_value::v2::ConcreteType;

    // The cache type is reachable and constructible.
    let mut cache = MonomorphizationCache::new();
    assert!(cache.is_empty(), "fresh cache must be empty");
    assert_eq!(cache.len(), 0);

    // build_mono_key produces the documented `<base>::<arg1>_<arg2>` shape.
    let key = build_mono_key("map", &[ConcreteType::I64, ConcreteType::String]);
    assert_eq!(key, "map::i64_string");

    // Insert/lookup round-trip.
    cache.insert(key.clone(), 7);
    assert_eq!(cache.lookup(&key), Some(7));
    assert_eq!(cache.len(), 1);

    // The compiler exposes a `monomorphization_cache` field — make sure the
    // type is the one we just exercised.
    let compiler = crate::compiler::BytecodeCompiler::new();
    let _: &MonomorphizationCache = &compiler.monomorphization_cache;
}

// ---------------------------------------------------------------------------
// Standalone ConcreteType::mono_key() semantics tests.
//
// These exercise [`shape_value::v2::ConcreteType::mono_key`] only — no
// dependency on Agent 1/2/3 APIs. They are the safety net that ensures the
// shape of mono keys we EXPECT in the gated tests below stays stable.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod mono_key_tests {
    use shape_value::v2::ConcreteType;

    /// `Array<int>` should produce a stable, recognisable key. The exact
    /// spelling matters because the gated tests below grep for substrings
    /// like `"i64"` and `"array_i64"` inside the cache.
    #[test]
    fn mono_key_for_array_of_int() {
        let arr_int = ConcreteType::Array(Box::new(ConcreteType::I64));
        let key = arr_int.mono_key();
        assert_eq!(key, "array_i64");
        assert!(key.contains("i64"));
        assert!(key.contains("array"));
    }

    /// Nested generic: `HashMap<string, Array<number>>`. Verifies that the
    /// recursive `mono_key` walker emits a fully-qualified composite name.
    #[test]
    fn mono_key_for_hashmap_string_to_array_of_number() {
        let inner = ConcreteType::Array(Box::new(ConcreteType::F64));
        let map_ty = ConcreteType::HashMap(Box::new(ConcreteType::String), Box::new(inner));
        let key = map_ty.mono_key();
        assert_eq!(key, "hashmap_string_array_f64");
        assert!(key.contains("string"));
        assert!(key.contains("array_f64"));
    }

    /// `int` (i64) and `number` (f64) MUST produce different keys — otherwise
    /// `map<int, _>` and `map<number, _>` would collide in the specialization
    /// cache and one would silently shadow the other.
    #[test]
    fn mono_key_disambiguates_int_vs_number() {
        let int_key = ConcreteType::I64.mono_key();
        let num_key = ConcreteType::F64.mono_key();
        assert_ne!(int_key, num_key);
        assert_eq!(int_key, "i64");
        assert_eq!(num_key, "f64");

        // And the same for the array forms — `Array<int>` ≠ `Array<number>`.
        let arr_int = ConcreteType::Array(Box::new(ConcreteType::I64)).mono_key();
        let arr_num = ConcreteType::Array(Box::new(ConcreteType::F64)).mono_key();
        assert_ne!(arr_int, arr_num);
    }
}

// ---------------------------------------------------------------------------
// End-to-end monomorphization tests.
//
// These tests compile Shape programs that exercise generic stdlib methods
// (map, filter, reduce, flatten) and verify that:
//   - The monomorphization cache keys in `BytecodeProgram::monomorphization_keys`
//     contain the expected specialization entries.
//   - Runtime results are correct end-to-end.
//
// Cache keys are transferred from `BytecodeCompiler::monomorphization_cache`
// into `BytecodeProgram::monomorphization_keys` at the end of compilation,
// since `compile()` consumes the compiler by value.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod e2e_tests {
    use shape_value::ValueWordExt;
    use crate::compiler::BytecodeCompiler;
    use crate::executor::{VMConfig, VirtualMachine};
    #[allow(unused_imports)]
    use crate::test_utils::eval_with_prelude;

    /// Helper: compile `source` with the prelude, return the bytecode program.
    /// Cache keys are available via `bytecode.monomorphization_keys`.
    fn compile_with_prelude(source: &str) -> crate::bytecode::BytecodeProgram {
        let program = shape_ast::parser::parse_program(source).expect("parse failed");
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        let (graph, stdlib_names, prelude_imports) =
            crate::module_resolution::build_graph_and_stdlib_names(&program, &mut loader, &[])
                .expect("graph build failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.stdlib_function_names = stdlib_names;
        compiler
            .compile_with_graph_and_prelude(&program, graph, &prelude_imports)
            .expect("compile failed")
    }

    fn run_program(bytecode: &crate::bytecode::BytecodeProgram) -> shape_value::ValueWord {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode.clone());
        vm.execute(None).expect("execution failed").clone()
    }

    /// `arr.map(|x| x + 1)` for `arr: Array<int>` should produce a
    /// `Vec.map::i64` entry in the specialization cache. The key encodes the
    /// receiver element type `T` only — the closure return type `U` is not a
    /// declared type parameter of the extend block.
    #[test]
    fn test_map_int_specialization() {
        let source = r#"
            let arr = [1, 2, 3]
            let result = arr.map(|x| x + 1)
            result
        "#;
        let bytecode = compile_with_prelude(source);
        let cache_keys = &bytecode.monomorphization_keys;
        assert!(
            cache_keys.iter().any(|k| k.contains("map") && k.contains("i64")),
            "expected a map specialization keyed on i64 in cache, got: {:?}",
            cache_keys
        );

        let result = run_program(&bytecode);
        let arr = result.as_any_array().expect("expected array result");
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.get_nb(0).and_then(|v| v.as_i64()), Some(2));
        assert_eq!(arr.get_nb(1).and_then(|v| v.as_i64()), Some(3));
        assert_eq!(arr.get_nb(2).and_then(|v| v.as_i64()), Some(4));
    }

    /// `arr.map(|x| x * 2.0)` for `arr: Array<number>` should produce a
    /// `Vec.map::f64` entry in the specialization cache.
    #[test]
    fn test_map_number_specialization() {
        let source = r#"
            let arr = [1.5, 2.7]
            let result = arr.map(|x| x * 2.0)
            result
        "#;
        let bytecode = compile_with_prelude(source);
        let cache_keys = &bytecode.monomorphization_keys;
        assert!(
            cache_keys.iter().any(|k| k.contains("map") && k.contains("f64")),
            "expected a map specialization keyed on f64 in cache, got: {:?}",
            cache_keys
        );
    }

    /// `arr.filter(|x| x % 2 == 0)` should produce `filter::i64` (or similar)
    /// in the cache and return `[2, 4]`.
    #[test]
    fn test_filter_preserves_type() {
        let source = r#"
            let arr = [1, 2, 3, 4, 5]
            let evens = arr.filter(|x| x % 2 == 0)
            evens
        "#;
        let bytecode = compile_with_prelude(source);
        let cache_keys = &bytecode.monomorphization_keys;
        assert!(
            cache_keys.iter().any(|k| k.contains("filter") && k.contains("i64")),
            "expected a filter<i64> specialization in cache, got: {:?}",
            cache_keys
        );

        let result = run_program(&bytecode);
        let arr = result.as_any_array().expect("expected array result");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr.get_nb(0).and_then(|v| v.as_i64()), Some(2));
        assert_eq!(arr.get_nb(1).and_then(|v| v.as_i64()), Some(4));
    }

    /// `arr.reduce(|acc, x| acc + x, 0)` should produce a scalar result `6`
    /// (`1 + 2 + 3`). Note: the stdlib signature is `reduce(f, init)` — the
    /// function comes first, then the initial accumulator value.
    #[test]
    fn test_reduce_to_scalar() {
        let source = r#"
            let arr = [1, 2, 3]
            let sum = arr.reduce(|acc, x| acc + x, 0)
            sum
        "#;
        let result = eval_with_prelude(source);
        assert_eq!(result.as_i64(), Some(6));
    }

    /// Two `map` calls on arrays of the same element type should produce
    /// exactly ONE specialization in the cache (cache de-duplication).
    #[test]
    fn test_two_callsites_same_type_share_specialization() {
        let source = r#"
            let arr1 = [1, 2, 3]
            let r1 = arr1.map(|x| x + 1)
            let arr2 = [10, 20, 30]
            let r2 = arr2.map(|x| x + 2)
            r1
        "#;
        let bytecode = compile_with_prelude(source);
        let map_specializations: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("map") && k.contains("i64"))
            .collect();
        assert_eq!(
            map_specializations.len(),
            1,
            "two map<i64> call sites should share one specialization, got: {:?}",
            map_specializations
        );
    }

    /// `map` on `Array<int>` and `map` on `Array<number>` should produce TWO
    /// distinct entries in the cache.
    #[test]
    fn test_two_callsites_different_types_different_specializations() {
        let source = r#"
            let arr_int = [1, 2, 3]
            let r1 = arr_int.map(|x| x + 1)
            let arr_num = [1.0, 2.0, 3.0]
            let r2 = arr_num.map(|x| x + 1.0)
            r1
        "#;
        let bytecode = compile_with_prelude(source);
        let map_specializations: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("map"))
            .collect();
        assert!(
            map_specializations.len() >= 2,
            "two distinct map specializations expected, got: {:?}",
            map_specializations
        );
        // And they must NOT be equal.
        let unique: std::collections::HashSet<&&String> = map_specializations.iter().collect();
        assert_eq!(unique.len(), map_specializations.len());
    }

    /// `nested.flatten()` for a `[[int]]` value verifies that the
    /// monomorphizer correctly tracks element types through nested generic
    /// types — the flatten specialization should be keyed on `i64`, not on
    /// `Array<i64>`.
    ///
    /// `flatten` is defined in `impl Iterable for Array` (a trait impl).
    /// Trait impl methods now get synthesized type params (Stage 2.6), enabling
    /// monomorphization. However, `flatten` goes through the extend path (Vec.flatten
    /// exists), and the monomorphized body calls `self.len()` via the delegating
    /// extend method, which causes recursion due to the empty-template-call issue.
    /// Gated until the delegating extend method recursion issue is resolved.
    #[test]
    #[ignore] // pre-existing: monomorphized extend methods calling self.len() overflow
    fn test_nested_generic_call() {
        let source = r#"
            let nested = [[1, 2], [3, 4]]
            let flat = nested.flatten()
            flat
        "#;
        let bytecode = compile_with_prelude(source);
        let cache_keys = &bytecode.monomorphization_keys;
        assert!(
            cache_keys.iter().any(|k| k.contains("flatten")),
            "expected a flatten specialization in cache, got: {:?}",
            cache_keys
        );

        let result = run_program(&bytecode);
        let arr = result.as_any_array().expect("expected array result");
        assert_eq!(arr.len(), 4);
    }

    /// User-defined `fn identity<T>(x: T) -> T` called with `int` and
    /// `string` should produce two specializations: `identity::i64` and
    /// `identity::string`.
    ///
    /// NOTE: Monomorphization of regular (non-method) generic function calls
    /// is not yet wired into the compiler — only method calls on receivers
    /// with known concrete types are monomorphized today. This test is gated
    /// until `try_monomorphize_function_call` is implemented.
    #[test]
    #[ignore] // regular function calls do not trigger monomorphization yet
    fn test_user_defined_generic_function() {
        let source = r#"
            fn identity<T>(x: T) -> T { x }
            let a = identity(42)
            let b = identity("hi")
            a
        "#;
        let bytecode = compile_with_prelude(source);
        let cache_keys = &bytecode.monomorphization_keys;
        let identity_specs: Vec<&String> = cache_keys
            .iter()
            .filter(|k| k.contains("identity"))
            .collect();
        assert!(
            identity_specs.len() >= 2,
            "expected two identity specializations (i64 and string), got: {:?}",
            identity_specs
        );
        assert!(
            identity_specs.iter().any(|k| k.contains("i64")),
            "missing identity::i64, got: {:?}",
            identity_specs
        );
        assert!(
            identity_specs.iter().any(|k| k.contains("string")),
            "missing identity::string, got: {:?}",
            identity_specs
        );
    }

    /// A non-generic `fn add(a: int, b: int) -> int` MUST NOT appear in the
    /// monomorphization cache — there is nothing to specialize.
    #[test]
    fn test_no_monomorphization_for_concrete_function() {
        let source = r#"
            fn add(a: int, b: int) -> int { a + b }
            add(1, 2)
        "#;
        let bytecode = compile_with_prelude(source);
        let cache_keys = &bytecode.monomorphization_keys;
        assert!(
            !cache_keys.iter().any(|k| k.contains("add")),
            "concrete function `add` should NOT be in the monomorphization cache, got: {:?}",
            cache_keys
        );
    }

    /// Tests that `impl Trait for Vec` methods with untyped parameters get
    /// monomorphized via synthesized type params (Stage 2.6).
    ///
    /// Uses `Vec` (not `Array`) because the dispatch system maps array literals
    /// to extend type "Vec". The trait method `contains(value)` has an untyped
    /// `value` parameter. The compiler synthesizes type param `T` from the Vec
    /// receiver, and monomorphization resolves `T` to `i64` at the call site.
    ///
    /// The method uses `for item in self` (not `self.len()`) to avoid the
    /// pre-existing delegating extend method recursion issue.
    #[test]
    fn test_impl_trait_method_monomorphization() {
        let source = r#"
            trait Searchable {
                contains(value): bool,
            }
            impl Searchable for Vec {
                method contains(value) {
                    for item in self {
                        if item == value { return true }
                    }
                    false
                }
            }
            let arr = [10, 20, 30]
            arr.contains(20)
        "#;
        let bytecode = compile_with_prelude(source);
        let cache_keys = &bytecode.monomorphization_keys;
        assert!(
            cache_keys
                .iter()
                .any(|k| k.contains("contains") && k.contains("i64")),
            "expected a contains specialization keyed on i64 in cache, got: {:?}",
            cache_keys
        );

        let result = run_program(&bytecode);
        assert_eq!(
            result.as_bool(),
            Some(true),
            "contains(20) on [10,20,30] should return true"
        );
    }

}
