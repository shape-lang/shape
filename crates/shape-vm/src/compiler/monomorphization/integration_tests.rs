//! End-to-end integration tests for monomorphized stdlib functions.
//!
//! **Owner**: Agent 4 of Phase 2.1.
//!
//! These tests exercise the full monomorphization pipeline: compile generic
//! stdlib calls (`map`, `filter`, `reduce`, …) for concrete element types and
//! verify that:
//!   1. The specialization cache contains the expected `mono_key` entries.
//!   2. Two call sites with identical type args share one specialization.
//!   3. Two call sites with different type args produce two specializations.
//!   4. Concrete (non-generic) functions are NOT cached.
//!   5. The runtime result is correct end-to-end.
//!
//! ## Status
//!
//! Phase 2.1 is being executed by four agents in parallel:
//!   - Agent 1 — `type_resolution` (call-site type-arg resolution)
//!   - Agent 2 — `substitution`   (FunctionDef cloning + type substitution)
//!   - Agent 3 — `cache`          (`MonomorphizationCache` + integration into
//!     `BytecodeCompiler`)
//!   - Agent 4 — this file        (integration tests)
//!
//! Most of the tests below depend on Agent 1, 2, and 3 having landed their
//! APIs. Until then they are gated behind `#[cfg(any())]` (which is `false`,
//! so the test bodies are excluded from the build but still parsed by tools).
//! Each gated test carries a `TODO` comment naming the missing API.
//!
//! Tests that DO compile today:
//!   - [`test_monomorphization_module_exists`] — meta-test that confirms the
//!     `monomorphization` module path is reachable from the test crate.
//!   - The `mono_key_*` tests under [`mono_key_tests`] — they use only
//!     [`shape_value::v2::ConcreteType`] which Phase 1 already shipped.

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
// All of the tests below are gated behind `#[cfg(any())]` until Phase 2.1
// agents 1, 2, and 3 land their APIs. The gate is `any()` (an empty disjunction
// = always-false), which excludes the bodies from the build but keeps them
// parseable by IDE tooling.
//
// To enable these tests once the APIs exist:
//   1. Remove the `#[cfg(any())]` from `mod gated_e2e_tests`.
//   2. If `compiler.monomorphization_cache` is named differently, rename it.
//   3. If the mono_key shape differs from the documented `map::i64_string`,
//      adjust the substring assertions.
// ---------------------------------------------------------------------------

// NOTE: imports are intentionally inside the gated block so the rest of the
// crate keeps building while these APIs do not exist.

#[cfg(any())]
mod gated_e2e_tests {
    use crate::compiler::BytecodeCompiler;
    use crate::executor::{VMConfig, VirtualMachine};
    use crate::test_utils::{compile_with_prelude, eval_with_prelude};

    /// Helper: compile `source` with the prelude, return the compiler so we
    /// can inspect [`BytecodeCompiler::monomorphization_cache`]. Replace this
    /// with whatever Agent 3 names the cache field.
    ///
    /// TODO(agent-3): exposes `BytecodeCompiler::monomorphization_cache` field.
    fn compile_and_inspect(
        source: &str,
    ) -> (BytecodeCompiler, crate::bytecode::BytecodeProgram) {
        let program = shape_ast::parser::parse_program(source).expect("parse failed");
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        let (graph, stdlib_names, prelude_imports) =
            crate::module_resolution::build_graph_and_stdlib_names(&program, &mut loader, &[])
                .expect("graph build failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.stdlib_function_names = stdlib_names;
        let bytecode = compiler
            .compile_with_graph_and_prelude(&program, graph, &prelude_imports)
            .expect("compile failed");
        (compiler, bytecode)
    }

    fn run_program(bytecode: crate::bytecode::BytecodeProgram) -> shape_value::ValueWord {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.execute(None).expect("execution failed").clone()
    }

    /// `arr.map(|x| x.toString())` for `arr: Array<int>` should produce a
    /// `map::i64_string` (or similar) entry in the specialization cache and
    /// return `["1", "2", "3"]` at runtime.
    ///
    /// TODO(agents 1-3): depends on full monomorphizer wired into the compiler.
    #[test]
    fn test_map_int_to_string() {
        let source = r#"
            let arr = [1, 2, 3]
            let result = arr.map(|x| x.toString())
            result
        "#;
        let (compiler, bytecode) = compile_and_inspect(source);
        let cache_keys: Vec<String> = compiler
            .monomorphization_cache
            .keys()
            .cloned()
            .collect();
        assert!(
            cache_keys.iter().any(|k| k.contains("map") && k.contains("i64") && k.contains("string")),
            "expected a map<i64, string> specialization in cache, got: {:?}",
            cache_keys
        );

        let result = run_program(bytecode);
        let arr = result.as_any_array().expect("expected array result");
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.get_nb(0).as_ref().and_then(|v| v.as_str().map(String::from)), Some("1".into()));
        assert_eq!(arr.get_nb(1).as_ref().and_then(|v| v.as_str().map(String::from)), Some("2".into()));
        assert_eq!(arr.get_nb(2).as_ref().and_then(|v| v.as_str().map(String::from)), Some("3".into()));
    }

    /// `arr.map(|x| x as int)` for `arr: Array<number>` should produce a
    /// `map::f64_i64` (or similar) entry in the specialization cache.
    ///
    /// TODO(agents 1-3): depends on full monomorphizer wired into the compiler.
    #[test]
    fn test_map_number_to_int() {
        let source = r#"
            let arr = [1.5, 2.7]
            let result = arr.map(|x| x as int)
            result
        "#;
        let (compiler, _) = compile_and_inspect(source);
        let cache_keys: Vec<String> = compiler
            .monomorphization_cache
            .keys()
            .cloned()
            .collect();
        assert!(
            cache_keys.iter().any(|k| k.contains("map") && k.contains("f64") && k.contains("i64")),
            "expected a map<f64, i64> specialization in cache, got: {:?}",
            cache_keys
        );
    }

    /// `arr.filter(|x| x % 2 == 0)` should produce `filter::i64` (or similar)
    /// in the cache and return `[2, 4]`.
    ///
    /// TODO(agents 1-3): depends on full monomorphizer wired into the compiler.
    #[test]
    fn test_filter_preserves_type() {
        let source = r#"
            let arr = [1, 2, 3, 4, 5]
            let evens = arr.filter(|x| x % 2 == 0)
            evens
        "#;
        let (compiler, bytecode) = compile_and_inspect(source);
        let cache_keys: Vec<String> = compiler
            .monomorphization_cache
            .keys()
            .cloned()
            .collect();
        assert!(
            cache_keys.iter().any(|k| k.contains("filter") && k.contains("i64")),
            "expected a filter<i64> specialization in cache, got: {:?}",
            cache_keys
        );

        let result = run_program(bytecode);
        let arr = result.as_any_array().expect("expected array result");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr.get_nb(0).and_then(|v| v.as_i64()), Some(2));
        assert_eq!(arr.get_nb(1).and_then(|v| v.as_i64()), Some(4));
    }

    /// `arr.reduce(0, |acc, x| acc + x)` should produce a scalar result `6`
    /// (`1 + 2 + 3`).
    ///
    /// TODO(agents 1-3): depends on full monomorphizer wired into the compiler.
    #[test]
    fn test_reduce_to_scalar() {
        let source = r#"
            let arr = [1, 2, 3]
            let sum = arr.reduce(0, |acc, x| acc + x)
            sum
        "#;
        let result = eval_with_prelude(source);
        assert_eq!(result.as_i64(), Some(6));
    }

    /// Two `map<int, string>` calls in different functions should produce
    /// exactly ONE specialization in the cache.
    ///
    /// TODO(agents 1-3): depends on cache de-duplication by mono_key.
    #[test]
    fn test_two_callsites_same_type_share_specialization() {
        let source = r#"
            fn f1() {
                let arr = [1, 2, 3]
                arr.map(|x| x.toString())
            }
            fn f2() {
                let arr = [10, 20, 30]
                arr.map(|x| x.toString())
            }
            f1()
            f2()
        "#;
        let (compiler, _) = compile_and_inspect(source);
        let map_specializations: Vec<String> = compiler
            .monomorphization_cache
            .keys()
            .filter(|k| k.contains("map") && k.contains("i64") && k.contains("string"))
            .cloned()
            .collect();
        assert_eq!(
            map_specializations.len(),
            1,
            "two map<i64, string> call sites should share one specialization, got: {:?}",
            map_specializations
        );
    }

    /// `map<int, string>` and `map<number, bool>` should produce TWO distinct
    /// entries in the cache.
    ///
    /// TODO(agents 1-3): depends on cache keying by full type signature.
    #[test]
    fn test_two_callsites_different_types_different_specializations() {
        let source = r#"
            fn f1() {
                let arr = [1, 2, 3]
                arr.map(|x| x.toString())
            }
            fn f2() {
                let arr = [1.0, 2.0, 3.0]
                arr.map(|x| x > 1.5)
            }
            f1()
            f2()
        "#;
        let (compiler, _) = compile_and_inspect(source);
        let map_specializations: Vec<String> = compiler
            .monomorphization_cache
            .keys()
            .filter(|k| k.contains("map"))
            .cloned()
            .collect();
        assert!(
            map_specializations.len() >= 2,
            "two distinct map specializations expected, got: {:?}",
            map_specializations
        );
        // And they must NOT be equal.
        let unique: std::collections::HashSet<&String> = map_specializations.iter().collect();
        assert_eq!(unique.len(), map_specializations.len());
    }

    /// `nested.flatten()` for a `[[int]]` value verifies that the
    /// monomorphizer correctly tracks element types through nested generic
    /// types — the flatten specialization should be keyed on `i64`, not on
    /// `Array<i64>`.
    ///
    /// TODO(agents 1-3): depends on monomorphizer following nested element types.
    #[test]
    fn test_nested_generic_call() {
        let source = r#"
            let nested = [[1, 2], [3, 4]]
            let flat = nested.flatten()
            flat
        "#;
        let (compiler, bytecode) = compile_and_inspect(source);
        let cache_keys: Vec<String> = compiler
            .monomorphization_cache
            .keys()
            .cloned()
            .collect();
        assert!(
            cache_keys.iter().any(|k| k.contains("flatten")),
            "expected a flatten specialization in cache, got: {:?}",
            cache_keys
        );

        let result = run_program(bytecode);
        let arr = result.as_any_array().expect("expected array result");
        assert_eq!(arr.len(), 4);
    }

    /// User-defined `fn identity<T>(x: T) -> T` called with `int` and
    /// `string` should produce two specializations: `identity::i64` and
    /// `identity::string`.
    ///
    /// TODO(agents 1-3): depends on user-defined generic specialization.
    #[test]
    fn test_user_defined_generic_function() {
        let source = r#"
            fn identity<T>(x: T) -> T { x }
            let a = identity(42)
            let b = identity("hi")
            a
        "#;
        let (compiler, _) = compile_and_inspect(source);
        let cache_keys: Vec<String> = compiler
            .monomorphization_cache
            .keys()
            .cloned()
            .collect();
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
    ///
    /// TODO(agents 1-3): depends on the cache excluding fully concrete functions.
    #[test]
    fn test_no_monomorphization_for_concrete_function() {
        let source = r#"
            fn add(a: int, b: int) -> int { a + b }
            add(1, 2)
        "#;
        let (compiler, _) = compile_and_inspect(source);
        let cache_keys: Vec<String> = compiler
            .monomorphization_cache
            .keys()
            .cloned()
            .collect();
        assert!(
            !cache_keys.iter().any(|k| k.contains("add")),
            "concrete function `add` should NOT be in the monomorphization cache, got: {:?}",
            cache_keys
        );
    }
}
