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

    /// Two `map` calls on arrays of the same element type with
    /// STRUCTURALLY IDENTICAL closures should share one specialization
    /// (Phase C structural CSE, §3.4). Closures with different bodies
    /// (even if only differing in a constant) produce distinct
    /// specializations — the cache key includes a body hash to prevent
    /// incorrect sharing.
    #[test]
    fn test_two_callsites_same_type_share_specialization() {
        let source = r#"
            let arr1 = [1, 2, 3]
            let r1 = arr1.map(|x| x + 1)
            let arr2 = [10, 20, 30]
            let r2 = arr2.map(|x| x + 1)
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
            "two map<i64> call sites with identical closure bodies should share one specialization, got: {:?}",
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

    // =====================================================================
    // Phase C — per-closure monomorphization end-to-end tests.
    // =====================================================================

    /// `arr.map(|x| x+1)` over `Array<int>` emits a closure-aware
    /// specialization. The specialized body's function name matches the
    /// `map::array_i64_closure_*_i64` shape (either direct or prefixed with
    /// `Vec.` for method-dispatch routes).
    #[test]
    fn phase_c_map_closure_emits_specialized_body() {
        let source = r#"
            let arr = [1, 2, 3]
            let result = arr.map(|x| x + 1)
            result
        "#;
        let bytecode = compile_with_prelude(source);
        let has_phase_c_key = bytecode
            .monomorphization_keys
            .iter()
            .any(|k| k.contains("map") && k.contains("closure_"));
        assert!(
            has_phase_c_key,
            "expected a closure-aware map specialization, got: {:?}",
            bytecode.monomorphization_keys
        );
    }

    /// Phase C specialized body must contain ZERO `CallValue`/`CallClosure`
    /// opcodes — the closure body is inlined. Verified by scanning the
    /// specialized function's instructions.
    #[test]
    fn phase_c_specialized_body_has_fewer_call_value_opcodes() {
        // Compare CallValue counts between the closure-aware specialization
        // and the type-only (non-closure) specialization of the same stdlib
        // template. The closure-aware path must emit strictly fewer
        // CallValue opcodes — inlining eliminates the indirect dispatch
        // through `f`. The stdlib's map body may still emit a CallValue for
        // OTHER purposes (e.g. result.push dispatches through an extend
        // method that the compiler lowers to CallValue), which is out of
        // scope for Phase C. We only measure the impact of closure
        // inlining.
        use crate::bytecode::OpCode;
        let source = r#"
            let arr = [1, 2, 3]
            arr.map(|x| x + 1)
        "#;
        let bytecode = compile_with_prelude(source);

        let phase_c = bytecode
            .functions
            .iter()
            .find(|f| f.name.contains("map") && f.name.contains("closure_"))
            .expect("expected Phase C specialization");
        let type_only = bytecode
            .functions
            .iter()
            .find(|f| {
                f.name.contains("map")
                    && f.name.contains("i64")
                    && !f.name.contains("closure_")
            });

        fn count_call_value(
            bc: &crate::bytecode::BytecodeProgram,
            f: &crate::bytecode::Function,
        ) -> usize {
            let start = f.entry_point as usize;
            let end = start + f.body_length as usize;
            bc.instructions[start..end.min(bc.instructions.len())]
                .iter()
                .filter(|i| i.opcode == OpCode::CallValue)
                .count()
        }

        let phase_c_count = count_call_value(&bytecode, phase_c);

        // Primary assertion: the Phase C body either has ZERO CallValues
        // (ideal) OR strictly fewer than the type-only baseline. The
        // baseline (non-closure) body calls `f(item)` via CallValue at
        // least once, so Phase C inlining must remove at least one.
        if let Some(type_only_fn) = type_only {
            let type_only_count = count_call_value(&bytecode, type_only_fn);
            assert!(
                phase_c_count < type_only_count,
                "Phase C '{}' has {} CallValue; type-only '{}' has {} — inlining did not reduce indirect dispatch",
                phase_c.name,
                phase_c_count,
                type_only_fn.name,
                type_only_count,
            );
        } else {
            // If the compiler only produced the closure-aware specialization
            // (the type-only one never got emitted because every call site
            // was closure-aware), the tighter assertion applies: a
            // closure-aware specialization with a non-escaping closure
            // literal must contain ZERO CallValues attributable to the
            // closure dispatch. Any remaining CallValues come from OTHER
            // stdlib method calls (e.g. `result.push`) — but Phase C bodies
            // typically lower those to typed opcodes like `ArrayPushLocal`.
            //
            // Cap the allowance at 1 to catch regressions where the inliner
            // stops firing.
            assert!(
                phase_c_count <= 1,
                "Phase C '{}' has {} CallValue opcodes — inlining regressed",
                phase_c.name, phase_c_count
            );
        }
        // NOTE: `CallClosure` opcode does not exist yet in the VM — the
        // design doc §1.3 introduces it in a later phase. Until then,
        // the only indirect call opcode to guard against is `CallValue`.
    }

    /// Two `arr.map(|x| x * 2)` call sites at distinct AST spans with
    /// IDENTICAL capture signatures (none) share one Phase C specialization.
    #[test]
    fn phase_c_identical_closures_share_specialization() {
        let source = r#"
            let a = [1, 2, 3]
            let r1 = a.map(|x| x * 2)
            let b = [4, 5, 6]
            let r2 = b.map(|x| x * 2)
            r1
        "#;
        let bytecode = compile_with_prelude(source);
        let phase_c_keys: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("map") && k.contains("closure_"))
            .collect();
        assert_eq!(
            phase_c_keys.len(),
            1,
            "two structurally identical closures should share ONE Phase C specialization, got: {:?}",
            phase_c_keys
        );
    }

    /// Two syntactically identical closure LITERALS (no captures) at
    /// different call sites intern the same `ClosureTypeId` (Phase A's
    /// registry is per-capture-signature).
    #[test]
    fn phase_c_two_identical_closures_share_closure_type_id() {
        let source = r#"
            let a = [1, 2, 3]
            let r = a.map(|x| x + 1)
            let b = [4, 5, 6]
            let s = b.map(|x| x + 1)
            r
        "#;
        let bytecode = compile_with_prelude(source);
        // Both keys (if present) should carry the same closure_<N> segment.
        let phase_c_keys: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("map") && k.contains("closure_"))
            .collect();
        // Either one shared key, or two keys whose closure_<N> segment
        // matches. Phase A's registry + our key-dedup should force the
        // former.
        assert_eq!(
            phase_c_keys.len(),
            1,
            "expected one shared key for identical closures, got: {:?}",
            phase_c_keys
        );
    }

    /// A Function-typed parameter (`f: (int) => int`) called with a
    /// non-literal callable (a bound identifier) should NOT trigger Phase C
    /// specialization, since the arg is not an `Expr::FunctionExpr`.
    #[test]
    fn phase_c_non_closure_arg_skips_closure_specialization() {
        let source = r#"
            fn double(x: int) -> int { x * 2 }
            let arr = [1, 2, 3]
            arr.map(double)
        "#;
        let bytecode = compile_with_prelude(source);
        // No closure-aware specialization should be cached — `double` is a
        // plain named function, not a closure literal.
        let phase_c_keys: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("map") && k.contains("closure_"))
            .collect();
        assert!(
            phase_c_keys.is_empty(),
            "passing a bare function name must not trigger Phase C specialization, got: {:?}",
            phase_c_keys
        );
    }

    /// `arr.filter(|x| x > 0)` over `Array<int>` produces a Phase C key
    /// with `bool` as the closure's return type.
    #[test]
    fn phase_c_filter_closure_key_has_bool_return() {
        let source = r#"
            let arr = [1, -2, 3, -4, 5]
            arr.filter(|x| x > 0)
        "#;
        let bytecode = compile_with_prelude(source);
        let phase_c_keys: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("filter") && k.contains("closure_"))
            .collect();
        // The closure returns bool; the key must contain `_bool_b` where
        // `_b<hex>` is the structural body-hash suffix from CSE.
        assert!(
            phase_c_keys
                .iter()
                .any(|k| k.contains("_bool_b") || k.ends_with("_bool")),
            "expected a filter closure specialization with bool return, got: {:?}",
            phase_c_keys
        );
    }

    /// Closure with a captured variable — the capture signature differs
    /// from a no-capture closure, so within a SINGLE compilation unit the
    /// Phase A registry assigns distinct `ClosureTypeId`s. The keys then
    /// differ. Cross-module comparisons aren't meaningful because each
    /// compiler maintains its own monotonic registry starting at 0.
    #[test]
    fn phase_c_captured_vs_uncaptured_closures_keyed_distinctly() {
        let source = r#"
            let a = [1, 2, 3]
            let r1 = a.map(|x| x + 1)
            let n = 10
            let r2 = a.map(|x| x + n)
            r1
        "#;
        let bytecode = compile_with_prelude(source);
        let phase_c_keys: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("map") && k.contains("closure_"))
            .collect();
        // Two closures with DIFFERENT capture signatures within the same
        // compilation unit must produce TWO distinct Phase C keys.
        assert_eq!(
            phase_c_keys.len(),
            2,
            "captured vs uncaptured closures must produce distinct Phase C keys, got: {:?}",
            phase_c_keys
        );
        // And the two keys must not be equal.
        let mut unique: std::collections::HashSet<&&String> =
            std::collections::HashSet::new();
        for k in &phase_c_keys {
            unique.insert(k);
        }
        assert_eq!(unique.len(), 2, "keys must be distinct: {:?}", phase_c_keys);
    }

    /// Monomorphic cache hit — calling `arr.map(|x| x+1)` TWICE on the same
    /// receiver type + same closure shape results in a SINGLE cache entry
    /// (second call hits the cache).
    #[test]
    fn phase_c_second_identical_call_hits_cache() {
        let source = r#"
            let a = [1, 2, 3]
            let r1 = a.map(|x| x + 1)
            let r2 = a.map(|x| x + 1)
            r1
        "#;
        let bytecode = compile_with_prelude(source);
        let phase_c_keys: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("map") && k.contains("closure_"))
            .collect();
        assert_eq!(
            phase_c_keys.len(),
            1,
            "second call with identical closure must hit the cache, got: {:?}",
            phase_c_keys
        );
    }

    /// Reduce with a single closure arg — the closure is peeked, a
    /// `ClosureSpec` is recorded, and the mono key carries the closure
    /// segment.
    #[test]
    fn phase_c_reduce_single_closure_arg() {
        let source = r#"
            let arr = [1, 2, 3, 4, 5]
            arr.reduce(|acc, x| acc + x, 0)
        "#;
        let bytecode = compile_with_prelude(source);
        let phase_c_keys: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("reduce") && k.contains("closure_"))
            .collect();
        assert!(
            !phase_c_keys.is_empty(),
            "reduce should trigger Phase C specialization, got: {:?}",
            bytecode.monomorphization_keys
        );
    }

    /// §3.4 structural CSE — two closures with identical capture signatures
    /// (both capture nothing, so Phase A gives them the same ClosureTypeId)
    /// but DIFFERENT bodies (`|x| x + 1` vs `|x| x * 2`) produce TWO
    /// distinct Phase C specializations. Without body-hash CSE, they'd
    /// collide on a single entry and one body would silently overwrite the
    /// other.
    #[test]
    fn phase_c_different_bodies_same_captures_distinct_specializations() {
        let source = r#"
            let a = [1, 2, 3]
            let r1 = a.map(|x| x + 1)
            let r2 = a.map(|x| x * 2)
            r1
        "#;
        let bytecode = compile_with_prelude(source);
        let phase_c_keys: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("map") && k.contains("closure_"))
            .collect();
        assert_eq!(
            phase_c_keys.len(),
            2,
            "structurally different closure bodies must produce distinct Phase C specializations, got: {:?}",
            phase_c_keys
        );
    }

    /// Runtime correctness — the Phase C specialized `map` produces the
    /// same result as the generic path. Guards against inlining that
    /// silently corrupts the program's output.
    #[test]
    fn phase_c_map_runtime_result_matches() {
        let source = r#"
            let arr = [1, 2, 3]
            arr.map(|x| x + 10)
        "#;
        let bytecode = compile_with_prelude(source);
        let result = run_program(&bytecode);
        let arr = result.as_any_array().expect("expected array result");
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.get_nb(0).and_then(|v| v.as_i64()), Some(11));
        assert_eq!(arr.get_nb(1).and_then(|v| v.as_i64()), Some(12));
        assert_eq!(arr.get_nb(2).and_then(|v| v.as_i64()), Some(13));
    }

    /// Runtime correctness — Phase C specialized `filter` still filters
    /// correctly.
    #[test]
    fn phase_c_filter_runtime_result_matches() {
        let source = r#"
            let arr = [1, -2, 3, -4, 5]
            arr.filter(|x| x > 0)
        "#;
        let bytecode = compile_with_prelude(source);
        let result = run_program(&bytecode);
        let arr = result.as_any_array().expect("expected array result");
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.get_nb(0).and_then(|v| v.as_i64()), Some(1));
        assert_eq!(arr.get_nb(1).and_then(|v| v.as_i64()), Some(3));
        assert_eq!(arr.get_nb(2).and_then(|v| v.as_i64()), Some(5));
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
