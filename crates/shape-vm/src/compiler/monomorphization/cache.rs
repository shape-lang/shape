//! Specialization cache for monomorphized generic functions.
//!
//! Each generic function (`fn map<T, U>(arr: Array<T>, f: (T) -> U) -> Array<U>`)
//! is compiled lazily, once per concrete type-argument tuple it is called with.
//! The cache maps a stable [`mono_key`](#mono_key-format) string to the index
//! of the compiled specialization in
//! [`crate::bytecode::BytecodeProgram::functions`].
//!
//! # mono_key format
//!
//! The key is `"<base_fn_name>::<type1>_<type2>_..."`, where each `typeN` is
//! the result of [`shape_value::v2::ConcreteType::mono_key`]. Examples:
//!
//! - `"identity::i64"`
//! - `"map::i64_string"`
//! - `"reduce::array_f64_i64"`
//!
//! Use [`build_mono_key`] to construct keys consistently across the compiler.

use shape_ast::error::{Result, ShapeError};
use shape_value::v2::ConcreteType;
use std::collections::HashMap;

use crate::compiler::BytecodeCompiler;
use crate::compiler::monomorphization::substitution;
use crate::compiler::monomorphization::type_resolution::{
    ComptimeConstValue, build_mono_key_with_consts,
};

/// Cache mapping a monomorphization key to the compiled function index.
///
/// The cache is owned by [`crate::compiler::BytecodeCompiler`] and lives for
/// the duration of one compilation session. It is parallel to (not a
/// replacement for) `const_specializations`, which keys on compile-time const
/// argument values.
#[derive(Debug, Default, Clone)]
pub struct MonomorphizationCache {
    entries: HashMap<String, u16>,
}

impl MonomorphizationCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Look up a previously specialized function by its mono key.
    ///
    /// Returns the function index in `BytecodeProgram::functions`, or `None`
    /// if no specialization has been recorded for this key yet.
    pub fn lookup(&self, mono_key: &str) -> Option<u16> {
        self.entries.get(mono_key).copied()
    }

    /// Record that the function compiled at `function_idx` is the
    /// specialization for `mono_key`.
    ///
    /// If the key already exists, the existing entry is overwritten — callers
    /// should `lookup` first if they need to detect duplicates.
    pub fn insert(&mut self, mono_key: String, function_idx: u16) {
        self.entries.insert(mono_key, function_idx);
    }

    /// Number of distinct specializations currently cached.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over `(mono_key, function_idx)` pairs.
    ///
    /// Useful for diagnostics and incremental-compilation snapshots.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &u16)> {
        self.entries.iter()
    }

    /// Iterate over the cached `mono_key` strings.
    ///
    /// Used by diagnostics and by Agent 4's integration tests, which assert
    /// that specific keys land in the cache after compiling generic call
    /// sites.
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }
}

/// Construct a monomorphization key from a base function name and a tuple of
/// concrete type arguments.
///
/// The key shape is `"<base_fn_name>::<ct1>_<ct2>_..."`. With no type
/// arguments the key is just the base name.
///
/// This is a thin wrapper over
/// [`crate::compiler::monomorphization::type_resolution::build_mono_key_with_consts`]
/// passing an empty const-args slice. The two functions are guaranteed
/// byte-for-byte identical for type-only inputs.
pub fn build_mono_key(base_fn_name: &str, type_args: &[ConcreteType]) -> String {
    build_mono_key_with_consts(base_fn_name, type_args, &[])
}

impl BytecodeCompiler {
    /// Ensure a monomorphized specialization of `base_fn_name` for the given
    /// concrete type arguments exists in the bytecode program. Returns the
    /// function index of the specialized function.
    ///
    /// On a cache hit, this is a constant-time lookup.
    ///
    /// On a cache miss, the original `FunctionDef` is fetched from
    /// `function_defs`, cloned, and handed to Agent 2's substitution helpers
    /// to produce a type-specialized clone. The clone is then registered and
    /// compiled via the normal pipeline, and its index is recorded in the
    /// cache before being returned.
    ///
    /// `type_args` is a positional list aligned to the callee's declared
    /// `type_params`: `type_args[i]` binds `def.type_params[i]`. If the
    /// callee declares no type parameters or the arity does not match, an
    /// error is returned.
    ///
    /// # Errors
    ///
    /// - `Err(...)` if `base_fn_name` is not a known function in the current
    ///   compiler state.
    /// - `Err(...)` if the callee declares no type parameters but type
    ///   arguments were supplied.
    /// - `Err(...)` if `type_args.len()` does not match the number of declared
    ///   type parameters.
    /// - Any compile error returned by `compile_function` for the
    ///   substituted body.
    pub fn ensure_monomorphic_function(
        &mut self,
        base_fn_name: &str,
        type_args: &[ConcreteType],
    ) -> Result<u16> {
        let mono_key = build_mono_key(base_fn_name, type_args);

        if let Some(existing) = self.monomorphization_cache.lookup(&mono_key) {
            return Ok(existing);
        }

        // Look up the original FunctionDef AST. The bytecode compiler always
        // populates `function_defs` during `register_function`, so this is
        // the canonical store for substitution input.
        let original_def = self.function_defs.get(base_fn_name).cloned().ok_or_else(|| {
            ShapeError::SemanticError {
                message: format!(
                    "ensure_monomorphic_function: no FunctionDef AST recorded for '{}'",
                    base_fn_name
                ),
                location: None,
            }
        })?;

        // Build the {type-param-name -> ConcreteType} substitution map that
        // Agent 2's `substitute_function_def` consumes. This requires the
        // callee's declared `type_params` to align positionally with the
        // supplied `type_args`.
        let declared_type_params: Vec<String> = original_def
            .type_params
            .as_ref()
            .map(|tps| tps.iter().map(|tp| tp.name.clone()).collect())
            .unwrap_or_default();

        if declared_type_params.is_empty() {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "ensure_monomorphic_function: '{}' declares no type parameters but {} type arguments were supplied",
                    base_fn_name,
                    type_args.len()
                ),
                location: None,
            });
        }
        if declared_type_params.len() != type_args.len() {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "ensure_monomorphic_function: '{}' declares {} type parameters but {} type arguments were supplied",
                    base_fn_name,
                    declared_type_params.len(),
                    type_args.len()
                ),
                location: None,
            });
        }

        let subs: HashMap<String, ConcreteType> = declared_type_params
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();

        // Substitute type parameters throughout the cloned AST. Agent 2's
        // helper renames the function deterministically using
        // `mono_key_from_subs`, so the new name is unique per (base, subs).
        let specialized_def = substitution::substitute_function_def(&original_def, &subs);
        let specialized_name = specialized_def.name.clone();

        // Register the new function definition in the program. This populates
        // `function_defs`, `function_arity_bounds`, etc.
        self.register_function(&specialized_def)?;
        let specialization_idx_usize =
            self.find_function(&specialized_name).ok_or_else(|| {
                ShapeError::SemanticError {
                    message: format!(
                        "ensure_monomorphic_function: failed to register specialization '{}'",
                        specialized_name
                    ),
                    location: None,
                }
            })?;
        let specialization_idx: u16 =
            specialization_idx_usize.try_into().map_err(|_| ShapeError::SemanticError {
                message: format!(
                    "ensure_monomorphic_function: function index {} for '{}' overflows u16",
                    specialization_idx_usize, specialized_name
                ),
                location: None,
            })?;

        // Cache the index BEFORE compiling the body so recursive calls inside
        // the specialized function self-reference the same cache entry instead
        // of recursively re-monomorphizing.
        self.monomorphization_cache
            .insert(mono_key, specialization_idx);
        self.next_monomorphization_id = self.next_monomorphization_id.saturating_add(1);

        // Compile the specialized body. On failure, surface the error — the
        // caller is responsible for falling through to the generic path on
        // any failure mode it wants to tolerate.
        self.compile_function(&specialized_def)?;

        Ok(specialization_idx)
    }

    /// Ensure a monomorphized specialization of `base_fn_name` for the given
    /// type AND const generic arguments exists. Returns the function index of
    /// the specialized function.
    ///
    /// This is the const-generic-aware sibling of
    /// [`Self::ensure_monomorphic_function`]. The cache key incorporates both
    /// the type args and the const args (see [`build_mono_key_with_consts`]),
    /// so the same callee specialised twice with different `N` values
    /// (`repeat<3>` vs `repeat<5>`) produces two cache entries; specialised
    /// twice with the same `N` produces one. The same applies to mixed
    /// type+const generic functions (`fn matrix<T, const ROWS: int>(...)`).
    ///
    /// **Grammar gap**: as of Phase 5 the grammar does not yet allow declaring
    /// const generic params, so the `const_args` slice is empty for every
    /// real call site. This entry point exists so that the cache + naming +
    /// substitution path is exercised by tests today and is ready to wire up
    /// the moment the grammar adds `<const N: int>`.
    ///
    /// On a cache hit, this is a constant-time lookup.
    ///
    /// On a cache miss, the original `FunctionDef` is fetched from
    /// `function_defs`, cloned, and substituted by
    /// [`substitution::substitute_function_def_with_consts`]. The clone is
    /// then registered, compiled, and recorded in the cache.
    ///
    /// `type_args` is positional against `def.type_params` (the type-kind
    /// generics). `const_args` is positional against the const-kind generics
    /// in declaration order. When the grammar exposes a way to mark a generic
    /// as `const`, the alignment logic here will need to interleave them
    /// correctly — see the TODO inside the body.
    ///
    /// # Errors
    ///
    /// Same as [`Self::ensure_monomorphic_function`], plus:
    ///
    /// - `Err(...)` if the type args don't satisfy the same arity / presence
    ///   constraints as the type-only path.
    pub fn ensure_monomorphic_function_with_consts(
        &mut self,
        base_fn_name: &str,
        type_args: &[ConcreteType],
        const_args: &[ComptimeConstValue],
    ) -> Result<u16> {
        // Fast path: no const args at all → reuse the existing entry point so
        // type-only callers stay byte-for-byte identical.
        if const_args.is_empty() {
            return self.ensure_monomorphic_function(base_fn_name, type_args);
        }

        let mono_key = build_mono_key_with_consts(base_fn_name, type_args, const_args);

        if let Some(existing) = self.monomorphization_cache.lookup(&mono_key) {
            return Ok(existing);
        }

        let original_def = self.function_defs.get(base_fn_name).cloned().ok_or_else(|| {
            ShapeError::SemanticError {
                message: format!(
                    "ensure_monomorphic_function_with_consts: no FunctionDef AST recorded for '{}'",
                    base_fn_name
                ),
                location: None,
            }
        })?;

        // Pull the type-kind generic param names off the original def. The
        // grammar currently produces ONLY type-kind generics, so all entries
        // here are type params. When const generics land in the grammar, the
        // type_params vec will need a `kind` field and we'll have to filter
        // type-vs-const here. For now: every declared param is a type param.
        //
        // TODO(grammar-const-generics): once `TypeParam` carries a `kind`
        // discriminator, partition into `type_param_names` (positional against
        // `type_args`) and `const_param_names` (positional against
        // `const_args`).
        let type_param_names: Vec<String> = original_def
            .type_params
            .as_ref()
            .map(|tps| tps.iter().map(|tp| tp.name.clone()).collect())
            .unwrap_or_default();

        if type_param_names.len() != type_args.len() {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "ensure_monomorphic_function_with_consts: '{}' declares {} type parameters but {} type arguments were supplied",
                    base_fn_name,
                    type_param_names.len(),
                    type_args.len()
                ),
                location: None,
            });
        }

        let type_subs: HashMap<String, ConcreteType> = type_param_names
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();

        // The const_subs map is intentionally indexed by *position* until the
        // grammar gives const generic params real names. We synthesise
        // placeholder names `__const_<i>` so the substitution path has
        // something to look up.
        //
        // TODO(grammar-const-generics): replace these synthesised names with
        // the real declared names from the (future) const-typed entries in
        // `def.type_params`.
        let const_subs: HashMap<String, ComptimeConstValue> = const_args
            .iter()
            .enumerate()
            .map(|(i, v)| (format!("__const_{}", i), v.clone()))
            .collect();

        let specialized_def = substitution::substitute_function_def_with_consts(
            &original_def,
            &type_subs,
            &const_subs,
            &mono_key,
        );
        let specialized_name = specialized_def.name.clone();

        self.register_function(&specialized_def)?;
        let specialization_idx_usize =
            self.find_function(&specialized_name).ok_or_else(|| {
                ShapeError::SemanticError {
                    message: format!(
                        "ensure_monomorphic_function_with_consts: failed to register specialization '{}'",
                        specialized_name
                    ),
                    location: None,
                }
            })?;
        let specialization_idx: u16 =
            specialization_idx_usize.try_into().map_err(|_| ShapeError::SemanticError {
                message: format!(
                    "ensure_monomorphic_function_with_consts: function index {} for '{}' overflows u16",
                    specialization_idx_usize, specialized_name
                ),
                location: None,
            })?;

        // Cache BEFORE compiling so a recursive const-generic call inside
        // the body resolves through the cache instead of recursively
        // re-monomorphizing.
        self.monomorphization_cache
            .insert(mono_key, specialization_idx);
        self.next_monomorphization_id = self.next_monomorphization_id.saturating_add(1);

        self.compile_function(&specialized_def)?;

        Ok(specialization_idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cache_lookup_returns_none() {
        let cache = MonomorphizationCache::new();
        assert_eq!(cache.lookup("map::i64_string"), None);
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn insert_then_lookup_returns_index() {
        let mut cache = MonomorphizationCache::new();
        cache.insert("map::i64_string".to_string(), 42);
        assert_eq!(cache.lookup("map::i64_string"), Some(42));
        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());
    }

    #[test]
    fn multiple_instantiations_produce_distinct_keys() {
        let mut cache = MonomorphizationCache::new();

        let key_int_string = build_mono_key(
            "map",
            &[ConcreteType::I64, ConcreteType::String],
        );
        let key_f64_bool = build_mono_key("map", &[ConcreteType::F64, ConcreteType::Bool]);
        let key_array_f64 = build_mono_key(
            "map",
            &[
                ConcreteType::Array(Box::new(ConcreteType::F64)),
                ConcreteType::I64,
            ],
        );

        // Sanity-check the key shapes match the design doc.
        assert_eq!(key_int_string, "map::i64_string");
        assert_eq!(key_f64_bool, "map::f64_bool");
        assert_eq!(key_array_f64, "map::array_f64_i64");

        cache.insert(key_int_string.clone(), 1);
        cache.insert(key_f64_bool.clone(), 2);
        cache.insert(key_array_f64.clone(), 3);

        assert_eq!(cache.lookup(&key_int_string), Some(1));
        assert_eq!(cache.lookup(&key_f64_bool), Some(2));
        assert_eq!(cache.lookup(&key_array_f64), Some(3));
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn build_mono_key_no_type_args() {
        // A "monomorphization" of a non-generic function: just the base name.
        assert_eq!(build_mono_key("foo", &[]), "foo");
    }

    #[test]
    fn build_mono_key_single_type_arg() {
        assert_eq!(
            build_mono_key("identity", &[ConcreteType::I64]),
            "identity::i64"
        );
    }

    #[test]
    fn insert_overwrites_existing_key() {
        let mut cache = MonomorphizationCache::new();
        cache.insert("identity::i64".to_string(), 5);
        cache.insert("identity::i64".to_string(), 9);
        assert_eq!(cache.lookup("identity::i64"), Some(9));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn iter_yields_inserted_pairs() {
        let mut cache = MonomorphizationCache::new();
        cache.insert("a::i64".to_string(), 1);
        cache.insert("b::f64".to_string(), 2);

        let mut collected: Vec<(String, u16)> =
            cache.iter().map(|(k, v)| (k.clone(), *v)).collect();
        collected.sort();
        assert_eq!(
            collected,
            vec![("a::i64".to_string(), 1), ("b::f64".to_string(), 2)]
        );
    }

    #[test]
    fn ensure_monomorphic_function_unknown_name_errors() {
        let mut compiler = BytecodeCompiler::new();
        let result = compiler.ensure_monomorphic_function(
            "definitely_not_a_function",
            &[ConcreteType::I64],
        );
        assert!(result.is_err(), "expected error for unknown function name");
        let msg = format!("{:?}", result.err().unwrap());
        assert!(
            msg.contains("no FunctionDef AST recorded"),
            "expected unknown-function error, got: {}",
            msg
        );
    }

    // ---- Const generic cache tests ---------------------------------------
    //
    // These tests verify the de-duplication / distinctness behaviour of the
    // monomorphization cache when const generic args are involved. They use
    // raw cache.insert/lookup so they don't depend on the (still missing)
    // grammar surface for `<const N: int>`.

    #[test]
    fn const_generic_repeat_n_3_caches_one_entry() {
        // Simulate "repeat<3>(...)": the call site builds a mono_key via
        // build_mono_key_with_consts and inserts a specialization index.
        let mut cache = MonomorphizationCache::new();
        let key = build_mono_key_with_consts(
            "repeat",
            &[],
            &[ComptimeConstValue::Int(3)],
        );
        assert_eq!(key, "repeat::int_3");
        cache.insert(key.clone(), 11);
        assert_eq!(cache.lookup(&key), Some(11));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn const_generic_repeat_n_3_and_n_5_produce_two_entries() {
        let mut cache = MonomorphizationCache::new();
        let k3 = build_mono_key_with_consts(
            "repeat",
            &[],
            &[ComptimeConstValue::Int(3)],
        );
        let k5 = build_mono_key_with_consts(
            "repeat",
            &[],
            &[ComptimeConstValue::Int(5)],
        );
        assert_ne!(k3, k5);
        cache.insert(k3.clone(), 11);
        cache.insert(k5.clone(), 12);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.lookup(&k3), Some(11));
        assert_eq!(cache.lookup(&k5), Some(12));
    }

    #[test]
    fn const_generic_repeat_n_3_twice_collapses_to_one_entry() {
        // Two calls to repeat<3> should hit the SAME cache entry. We model
        // that by inserting twice with the same key and verifying the cache
        // length never grows past 1 (and the second insert overwrites).
        let mut cache = MonomorphizationCache::new();
        let key = build_mono_key_with_consts(
            "repeat",
            &[],
            &[ComptimeConstValue::Int(3)],
        );
        cache.insert(key.clone(), 11);
        cache.insert(key.clone(), 11);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.lookup(&key), Some(11));
    }

    #[test]
    fn ensure_monomorphic_function_with_consts_unknown_name_errors() {
        let mut compiler = BytecodeCompiler::new();
        let result = compiler.ensure_monomorphic_function_with_consts(
            "definitely_not_a_function",
            &[],
            &[ComptimeConstValue::Int(3)],
        );
        assert!(
            result.is_err(),
            "expected error for unknown function name even on the const-aware path"
        );
        let msg = format!("{:?}", result.err().unwrap());
        assert!(
            msg.contains("no FunctionDef AST recorded"),
            "expected unknown-function error, got: {}",
            msg
        );
    }

    #[test]
    fn ensure_monomorphic_function_with_consts_empty_consts_delegates_to_legacy_path() {
        // With no const args supplied, the const-aware entry point must
        // delegate to ensure_monomorphic_function (the type-only path) so
        // every existing caller stays byte-for-byte identical.
        //
        // We verify this by passing an unknown function name and checking
        // the error message — the legacy path's error string is distinct
        // from the const-aware one.
        let mut compiler = BytecodeCompiler::new();
        let result = compiler.ensure_monomorphic_function_with_consts(
            "definitely_not_a_function",
            &[ConcreteType::I64],
            &[], // empty const args → must delegate
        );
        assert!(result.is_err());
        let msg = format!("{:?}", result.err().unwrap());
        // Legacy error string ("ensure_monomorphic_function: ...") not the
        // const-aware one ("ensure_monomorphic_function_with_consts: ...").
        assert!(
            msg.contains("ensure_monomorphic_function:"),
            "expected delegation to legacy path, got: {}",
            msg
        );
        assert!(
            !msg.contains("ensure_monomorphic_function_with_consts:"),
            "should NOT have used the const-aware error path: {}",
            msg
        );
    }
}
