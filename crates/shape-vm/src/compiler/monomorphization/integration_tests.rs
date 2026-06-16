//! Integration tests for the monomorphization pipeline.
//!
//! Phase-2c R8 C3 rebuild (2026-05-23) — ADR-006 §2.7.4 / §2.7.6 / Q8.
//!
//! These tests exercise the post-`ValueWord` integration surface:
//!
//!   1. Cache-API meta-test (canary that the module path resolves and
//!      `MonomorphizationCache` round-trips).
//!   2. Pure `ConcreteType::mono_key` shape tests (no runtime needed).
//!   3. End-to-end specialization tests — compile a Shape program, assert
//!      that `BytecodeProgram::monomorphization_keys` carries the expected
//!      `mono_key` entries, and reduce runtime results to a scalar inside
//!      the Shape program so the host can decode via `KindedSlot`'s
//!      §2.7.6 / Q8 single-discriminator accessors (`as_i64`, `as_bool`,
//!      …) without poking into heap arrays from Rust.
//!
//! ## Tests gated out of this rebuild
//!
//! - Anything that decoded `as_any_array()` on a heap-array result is
//!   refolded so the *Shape* program reduces to a scalar (sum / len /
//!   indexed-read) before returning. The ValueWord-era pattern of
//!   inspecting array elements directly from Rust is gone.
//! - `Phase C bytecode-inspection tests` (count `CallValue` opcodes in
//!   the specialized body) survive unchanged — they only inspect bytecode
//!   shape, never carrier values.
//! - `test_nested_generic_call` remains `#[ignore]`'d (pre-existing
//!   flatten monomorphization-cache population gap unrelated to phase-2c).

// ---------------------------------------------------------------------------
// Meta-test: confirm the monomorphization module is reachable.
// ---------------------------------------------------------------------------

/// Canary that the `crate::compiler::monomorphization` module path
/// resolves and the cache API round-trips.
#[test]
fn test_monomorphization_module_exists() {
    use crate::compiler::monomorphization::cache::{MonomorphizationCache, build_mono_key};
    use shape_value::v2::ConcreteType;

    let mut cache = MonomorphizationCache::new();
    assert!(cache.is_empty(), "fresh cache must be empty");
    assert_eq!(cache.len(), 0);

    let key = build_mono_key("map", &[ConcreteType::I64, ConcreteType::String]);
    assert_eq!(key, "map::i64_string");

    cache.insert(key.clone(), 7);
    assert_eq!(cache.lookup(&key), Some(7));
    assert_eq!(cache.len(), 1);

    let compiler = crate::compiler::BytecodeCompiler::new();
    let _: &MonomorphizationCache = &compiler.monomorphization_cache;
}

// ---------------------------------------------------------------------------
// Standalone `ConcreteType::mono_key()` semantics tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod mono_key_tests {
    use shape_value::v2::ConcreteType;

    /// `Array<int>` produces a stable, recognisable key. The exact
    /// spelling matters because the e2e tests below grep for substrings
    /// like `"i64"` and `"array_i64"`.
    #[test]
    fn mono_key_for_array_of_int() {
        let arr_int = ConcreteType::Array(Box::new(ConcreteType::I64));
        let key = arr_int.mono_key();
        assert_eq!(key, "array_i64");
        assert!(key.contains("i64"));
        assert!(key.contains("array"));
    }

    /// Nested generic: `HashMap<string, Array<number>>`.
    #[test]
    fn mono_key_for_hashmap_string_to_array_of_number() {
        let inner = ConcreteType::Array(Box::new(ConcreteType::F64));
        let map_ty = ConcreteType::HashMap(Box::new(ConcreteType::String), Box::new(inner));
        let key = map_ty.mono_key();
        assert_eq!(key, "hashmap_string_array_f64");
        assert!(key.contains("string"));
        assert!(key.contains("array_f64"));
    }

    /// `int` (i64) and `number` (f64) MUST produce distinct keys.
    #[test]
    fn mono_key_disambiguates_int_vs_number() {
        let int_key = ConcreteType::I64.mono_key();
        let num_key = ConcreteType::F64.mono_key();
        assert_ne!(int_key, num_key);
        assert_eq!(int_key, "i64");
        assert_eq!(num_key, "f64");

        let arr_int = ConcreteType::Array(Box::new(ConcreteType::I64)).mono_key();
        let arr_num = ConcreteType::Array(Box::new(ConcreteType::F64)).mono_key();
        assert_ne!(arr_int, arr_num);
    }
}

// ---------------------------------------------------------------------------
// End-to-end monomorphization tests.
//
// Phase-2c R8 C3 rebuild: instead of decoding heap arrays from Rust
// (the ValueWord `as_any_array()` pattern), each Shape program reduces
// its result to a scalar (`.sum()`, indexed read, `.len()`, etc.) so
// the host receives a `KindedSlot` whose §2.7.6 / Q8 accessor decodes
// without any host-tier marshal work.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod e2e_tests {
    use crate::test_utils::{compile_with_prelude, eval_with_prelude};

    /// `arr.map(|x| x + 1)` for `arr: Array<int>` should produce a
    /// `map` × `i64` entry in the specialization cache.
    #[test]
    fn test_map_int_specialization() {
        let source = r#"
            let arr = [1, 2, 3]
            let result = arr.map(|x| x + 1)
            result.sum()
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
        let cache_keys = &bytecode.monomorphization_keys;
        assert!(
            cache_keys
                .iter()
                .any(|k| k.contains("map") && k.contains("i64")),
            "expected a map specialization keyed on i64 in cache, got: {:?}",
            cache_keys
        );

        // Runtime correctness reduced to a scalar inside the Shape
        // program: 2 + 3 + 4 = 9.
        let result = eval_with_prelude(source);
        assert_eq!(result.as_i64(), Some(9));
    }

    /// `arr.map(|x| x * 2.0)` for `arr: Array<number>` should produce a
    /// `map` × `f64` entry in the specialization cache.
    #[test]
    fn test_map_number_specialization() {
        let source = r#"
            let arr = [1.5, 2.7]
            let result = arr.map(|x| x * 2.0)
            result
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
        let cache_keys = &bytecode.monomorphization_keys;
        assert!(
            cache_keys
                .iter()
                .any(|k| k.contains("map") && k.contains("f64")),
            "expected a map specialization keyed on f64 in cache, got: {:?}",
            cache_keys
        );
    }

    /// `arr.filter(|x| x % 2 == 0)` should produce `filter::i64` (or
    /// similar) in the cache; the result `[2, 4]` reduces to `2 + 4 = 6`.
    #[test]
    fn test_filter_preserves_type() {
        let source = r#"
            let arr = [1, 2, 3, 4, 5]
            let evens = arr.filter(|x| x % 2 == 0)
            evens.sum()
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
        let cache_keys = &bytecode.monomorphization_keys;
        assert!(
            cache_keys
                .iter()
                .any(|k| k.contains("filter") && k.contains("i64")),
            "expected a filter<i64> specialization in cache, got: {:?}",
            cache_keys
        );

        let result = eval_with_prelude(source);
        assert_eq!(result.as_i64(), Some(6));
    }

    /// `arr.reduce(|acc, x| acc + x, 0)` should produce a scalar `6`
    /// (`1 + 2 + 3`).
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
    /// STRUCTURALLY IDENTICAL closures should share one specialization.
    #[test]
    fn test_two_callsites_same_type_share_specialization() {
        let source = r#"
            let arr1 = [1, 2, 3]
            let r1 = arr1.map(|x| x + 1)
            let arr2 = [10, 20, 30]
            let r2 = arr2.map(|x| x + 1)
            r1.sum()
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
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

    /// `map` on `Array<int>` and `map` on `Array<number>` should produce
    /// TWO distinct cache entries.
    #[test]
    fn test_two_callsites_different_types_different_specializations() {
        let source = r#"
            let arr_int = [1, 2, 3]
            let r1 = arr_int.map(|x| x + 1)
            let arr_num = [1.0, 2.0, 3.0]
            let r2 = arr_num.map(|x| x + 1.0)
            r1.sum()
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
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
        let unique: std::collections::HashSet<&&String> = map_specializations.iter().collect();
        assert_eq!(unique.len(), map_specializations.len());
    }

    /// `nested.flatten()` for `[[int]]` — pre-existing flatten
    /// specialization-cache population gap (unrelated to phase-2c).
    #[test]
    #[ignore]
    fn test_nested_generic_call() {
        let source = r#"
            let nested = [[1, 2], [3, 4]]
            let flat = nested.flatten()
            flat.sum()
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
        let cache_keys = &bytecode.monomorphization_keys;
        assert!(
            cache_keys.iter().any(|k| k.contains("flatten")),
            "expected a flatten specialization in cache, got: {:?}",
            cache_keys
        );

        let result = eval_with_prelude(source);
        // 1+2+3+4 = 10
        assert_eq!(result.as_i64(), Some(10));
    }

    /// User-defined `fn identity<T>(x: T) -> T` called with `int` and
    /// `string` should produce two specializations.
    #[test]
    fn test_user_defined_generic_function() {
        let source = r#"
            fn identity<T>(x: T) -> T { x }
            let a = identity(42)
            let b = identity("hi")
            a
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
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

    /// A non-generic `fn add(...)` MUST NOT appear in the
    /// monomorphization cache.
    #[test]
    fn test_no_monomorphization_for_concrete_function() {
        let source = r#"
            fn add(a: int, b: int) -> int { a + b }
            add(1, 2)
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
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

    /// `arr.map(|x| x+1)` emits a closure-aware specialization with the
    /// `closure_` segment in its key.
    #[test]
    fn phase_c_map_closure_emits_specialized_body() {
        let source = r#"
            let arr = [1, 2, 3]
            let result = arr.map(|x| x + 1)
            result.sum()
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
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

    /// Phase C specialized body must contain strictly fewer (or zero)
    /// `CallValue` opcodes than the type-only variant — the closure is
    /// inlined, eliminating the indirect dispatch through `f`.
    #[test]
    fn phase_c_specialized_body_has_fewer_call_value_opcodes() {
        use crate::bytecode::OpCode;
        let source = r#"
            let arr = [1, 2, 3]
            arr.map(|x| x + 1)
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");

        let phase_c = bytecode
            .functions
            .iter()
            .find(|f| f.name.contains("map") && f.name.contains("closure_"))
            .expect("expected Phase C specialization");
        let type_only = bytecode.functions.iter().find(|f| {
            f.name.contains("map") && f.name.contains("i64") && !f.name.contains("closure_")
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
            assert!(
                phase_c_count <= 1,
                "Phase C '{}' has {} CallValue opcodes — inlining regressed",
                phase_c.name,
                phase_c_count
            );
        }
    }

    /// Two `arr.map(|x| x * 2)` call sites with IDENTICAL capture
    /// signatures share one Phase C specialization.
    #[test]
    fn phase_c_identical_closures_share_specialization() {
        let source = r#"
            let a = [1, 2, 3]
            let r1 = a.map(|x| x * 2)
            let b = [4, 5, 6]
            let r2 = b.map(|x| x * 2)
            r1.sum()
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
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

    /// Two syntactically identical closure literals (no captures) at
    /// different call sites share the same `ClosureTypeId`.
    #[test]
    fn phase_c_two_identical_closures_share_closure_type_id() {
        let source = r#"
            let a = [1, 2, 3]
            let r = a.map(|x| x + 1)
            let b = [4, 5, 6]
            let s = b.map(|x| x + 1)
            r.sum()
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
        let phase_c_keys: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("map") && k.contains("closure_"))
            .collect();
        assert_eq!(
            phase_c_keys.len(),
            1,
            "expected one shared key for identical closures, got: {:?}",
            phase_c_keys
        );
    }

    /// A bound function name (not a closure literal) skips Phase C
    /// specialization.
    #[test]
    fn phase_c_non_closure_arg_skips_closure_specialization() {
        let source = r#"
            fn double(x: int) -> int { x * 2 }
            let arr = [1, 2, 3]
            arr.map(double)
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
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

    /// `arr.filter(|x| x > 0)` produces a Phase C key whose closure
    /// return type is `bool`.
    #[test]
    fn phase_c_filter_closure_key_has_bool_return() {
        let source = r#"
            let arr = [1, -2, 3, -4, 5]
            arr.filter(|x| x > 0)
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
        let phase_c_keys: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("filter") && k.contains("closure_"))
            .collect();
        assert!(
            phase_c_keys
                .iter()
                .any(|k| k.contains("_bool_b") || k.ends_with("_bool")),
            "expected a filter closure specialization with bool return, got: {:?}",
            phase_c_keys
        );
    }

    /// Captured vs uncaptured closures produce distinct Phase C keys.
    #[test]
    fn phase_c_captured_vs_uncaptured_closures_keyed_distinctly() {
        let source = r#"
            let a = [1, 2, 3]
            let r1 = a.map(|x| x + 1)
            let n = 10
            let r2 = a.map(|x| x + n)
            r1.sum()
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
        let phase_c_keys: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("map") && k.contains("closure_"))
            .collect();
        assert_eq!(
            phase_c_keys.len(),
            2,
            "captured vs uncaptured closures must produce distinct Phase C keys, got: {:?}",
            phase_c_keys
        );
        let mut unique: std::collections::HashSet<&&String> = std::collections::HashSet::new();
        for k in &phase_c_keys {
            unique.insert(k);
        }
        assert_eq!(unique.len(), 2, "keys must be distinct: {:?}", phase_c_keys);
    }

    /// Calling `arr.map(|x| x+1)` twice with identical receiver type +
    /// closure shape results in ONE cache entry.
    #[test]
    fn phase_c_second_identical_call_hits_cache() {
        let source = r#"
            let a = [1, 2, 3]
            let r1 = a.map(|x| x + 1)
            let r2 = a.map(|x| x + 1)
            r1.sum()
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
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

    /// Reduce with a single closure arg triggers Phase C.
    #[test]
    fn phase_c_reduce_single_closure_arg() {
        let source = r#"
            let arr = [1, 2, 3, 4, 5]
            arr.reduce(|acc, x| acc + x, 0)
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
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

    /// §3.4 structural CSE — two closures with identical capture
    /// signatures but DIFFERENT bodies produce distinct Phase C
    /// specializations.
    #[test]
    fn phase_c_different_bodies_same_captures_distinct_specializations() {
        let source = r#"
            let a = [1, 2, 3]
            let r1 = a.map(|x| x + 1)
            let r2 = a.map(|x| x * 2)
            r1.sum()
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
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

    /// Phase C specialized `map` produces the correct numerical result.
    /// Reduced to a scalar via `.sum()` so the host decodes via
    /// `KindedSlot::as_i64`.
    #[test]
    fn phase_c_map_runtime_result_matches() {
        let source = r#"
            let arr = [1, 2, 3]
            arr.map(|x| x + 10).sum()
        "#;
        // 11 + 12 + 13 = 36
        let result = eval_with_prelude(source);
        assert_eq!(result.as_i64(), Some(36));
    }

    /// Phase C specialized `filter` filters correctly.
    /// Reduced to a scalar via `.sum()`.
    #[test]
    fn phase_c_filter_runtime_result_matches() {
        let source = r#"
            let arr = [1, -2, 3, -4, 5]
            arr.filter(|x| x > 0).sum()
        "#;
        // 1 + 3 + 5 = 9
        let result = eval_with_prelude(source);
        assert_eq!(result.as_i64(), Some(9));
    }

    /// `impl Trait for Vec` methods with untyped parameters get
    /// monomorphized via synthesized type params (Stage 2.6).
    ///
    /// Phase-2c R8 C3 rebuild (2026-05-23): updated to current trait
    /// syntax — `method <name>(args) -> ReturnType;` rather than the
    /// pre-V1.3 `<name>(args): ReturnType,`.
    #[test]
    fn test_impl_trait_method_monomorphization() {
        let source = r#"
            trait Searchable {
                method has(value) -> bool;
            }
            impl Searchable for Vec {
                method has(value) -> bool {
                    for item in self {
                        if item == value { return true }
                    }
                    false
                }
            }
            let arr = [10, 20, 30]
            arr.has(20)
        "#;
        let bytecode = compile_with_prelude(source).expect("compile failed");
        let cache_keys = &bytecode.monomorphization_keys;
        assert!(
            cache_keys
                .iter()
                .any(|k| k.contains("has") && k.contains("i64")),
            "expected a has specialization keyed on i64 in cache, got: {:?}",
            cache_keys
        );

        let result = eval_with_prelude(source);
        assert_eq!(
            result.as_bool(),
            Some(true),
            "has(20) on [10,20,30] should return true"
        );
    }
}

// ---------------------------------------------------------------------------
// Phase 3a (Option β foundation) — trait-method dispatch via monomorphizing
// generics with trait bounds.
//
// These tests use `eval` / `eval_result` from `test_utils` (no prelude) so
// they exercise the bound-checking + monomorphization path in isolation
// from the stdlib.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod phase_3a_trait_bounds {
    use crate::bytecode::OpCode;
    use crate::compiler::BytecodeCompiler;
    use crate::test_utils::{eval, eval_result};

    /// `fn clamp<T: Ord>(x: T, lo: T, hi: T) -> T` is callable with both
    /// `int` and `number`. Each call site produces a distinct
    /// monomorphized specialization.
    #[test]
    fn clamp_with_ord_bound_specializes_for_int_and_number() {
        let source = r#"
            fn clamp<T: Ord>(x: T, lo: T, hi: T) -> T {
                if x < lo { lo }
                else if x > hi { hi }
                else { x }
            }

            let a = clamp(15, 0, 10)
            let b = clamp(0.5, 0.0, 1.0)
            a
        "#;

        let program = shape_ast::parser::parse_program(source).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        let bytecode = compiler.compile(&program).expect("compile failed");

        let cache_keys = &bytecode.monomorphization_keys;
        let clamp_specs: Vec<&String> = cache_keys.iter().filter(|k| k.contains("clamp")).collect();
        assert!(
            clamp_specs.iter().any(|k| k.contains("i64")),
            "missing clamp::i64 specialization, got: {:?}",
            clamp_specs
        );
        assert!(
            clamp_specs.iter().any(|k| k.contains("f64")),
            "missing clamp::f64 specialization, got: {:?}",
            clamp_specs
        );

        // Top-level value is `a` = clamp(15, 0, 10) → 10.
        let val = eval(source);
        assert_eq!(val.as_i64(), Some(10), "clamp(15, 0, 10) should be 10");
    }

    /// The monomorphized `clamp::i64` body contains `LtInt` / `GtInt`
    /// — direct typed ops, no trait-method indirection.
    #[test]
    fn specialized_body_uses_typed_int_compare_opcodes() {
        let source = r#"
            fn clamp<T: Ord>(x: T, lo: T, hi: T) -> T {
                if x < lo { lo }
                else if x > hi { hi }
                else { x }
            }
            clamp(15, 0, 10)
        "#;
        let program = shape_ast::parser::parse_program(source).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        let bytecode = compiler.compile(&program).expect("compile failed");

        let i64_idx = bytecode
            .monomorphization_keys
            .iter()
            .position(|k| k.contains("clamp") && k.contains("i64"));
        assert!(i64_idx.is_some(), "clamp::i64 must be in the cache");

        let spec_fn = bytecode
            .functions
            .iter()
            .find(|f| f.name.contains("clamp") && f.name.contains("i64"))
            .expect("clamp::i64 function not found");
        let body = &bytecode.instructions[spec_fn.entry_point as usize
            ..(spec_fn.entry_point as usize + spec_fn.body_length as usize)];
        let opcodes: Vec<OpCode> = body.iter().map(|ins| ins.opcode).collect();
        assert!(
            opcodes.iter().any(|op| matches!(op, OpCode::LtInt)),
            "specialized clamp::i64 body should emit LtInt; got: {:?}",
            opcodes
        );
        assert!(
            opcodes.iter().any(|op| matches!(op, OpCode::GtInt)),
            "specialized clamp::i64 body should emit GtInt; got: {:?}",
            opcodes
        );
    }

    /// `clamp::f64` body contains `LtNumber` / `GtNumber`.
    #[test]
    fn specialized_body_uses_typed_number_compare_opcodes() {
        let source = r#"
            fn clamp<T: Ord>(x: T, lo: T, hi: T) -> T {
                if x < lo { lo }
                else if x > hi { hi }
                else { x }
            }
            clamp(0.5, 0.0, 1.0)
        "#;
        let program = shape_ast::parser::parse_program(source).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        let bytecode = compiler.compile(&program).expect("compile failed");

        let spec_fn = bytecode
            .functions
            .iter()
            .find(|f| f.name.contains("clamp") && f.name.contains("f64"))
            .expect("clamp::f64 function not found");
        let body = &bytecode.instructions[spec_fn.entry_point as usize
            ..(spec_fn.entry_point as usize + spec_fn.body_length as usize)];
        let opcodes: Vec<OpCode> = body.iter().map(|ins| ins.opcode).collect();
        assert!(
            opcodes.iter().any(|op| matches!(op, OpCode::LtNumber)),
            "specialized clamp::f64 body should emit LtNumber; got: {:?}",
            opcodes
        );
        assert!(
            opcodes.iter().any(|op| matches!(op, OpCode::GtNumber)),
            "specialized clamp::f64 body should emit GtNumber; got: {:?}",
            opcodes
        );
    }

    /// Negative test: `<T: Iterable>` called with `int` must fail
    /// compilation with a precise diagnostic.
    #[test]
    fn calling_iterable_bounded_fn_with_int_fails() {
        let source = r#"
            fn require_iter<T: Iterable>(x: T) -> T { x }
            require_iter(42)
        "#;
        let result = eval_result(source);
        assert!(
            result.is_err(),
            "expected compile error: int does not impl Iterable"
        );
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("trait bound not satisfied")
                && msg.contains("int")
                && msg.contains("Iterable"),
            "expected bound-violation diagnostic, got: {msg}"
        );
    }

    /// Two int call sites share one `clamp::i64` specialization.
    #[test]
    fn two_int_callsites_share_one_specialization() {
        let source = r#"
            fn clamp<T: Ord>(x: T, lo: T, hi: T) -> T {
                if x < lo { lo }
                else if x > hi { hi }
                else { x }
            }
            let a = clamp(5, 0, 10)
            let b = clamp(20, 0, 10)
            a + b
        "#;
        let program = shape_ast::parser::parse_program(source).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        let bytecode = compiler.compile(&program).expect("compile failed");

        let i64_specs: Vec<&String> = bytecode
            .monomorphization_keys
            .iter()
            .filter(|k| k.contains("clamp") && k.contains("i64"))
            .collect();
        assert_eq!(
            i64_specs.len(),
            1,
            "two int call sites must share one clamp::i64 specialization, got: {:?}",
            i64_specs
        );

        // 5 + 10 == 15
        let val = eval(source);
        assert_eq!(val.as_i64(), Some(15));
    }
}
