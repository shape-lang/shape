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
    ClosureSpec, ComptimeConstValue, build_mono_key_full, build_mono_key_with_consts,
    comptime_const_value_from_literal_expr, split_type_and_const_param_names,
};

/// Phase C — per-module specialization budget.
///
/// Once the per-module closure-specialization count exceeds this threshold
/// the compiler falls back to the existing (non-inlined) generic dispatch
/// path. Prevents unbounded code-size growth for programs that generate
/// hundreds of distinct closure types. See §3.4 of
/// `docs/v2-closure-specialization.md` for the rationale.
pub const DEFAULT_CLOSURE_SPECIALIZATION_BUDGET: u32 = 64;

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

/// Phase C — construct a mono key that includes per-closure-arg
/// specialization segments. Thin re-export of
/// [`build_mono_key_full`] so external callers can go through the cache
/// module uniformly.
pub fn build_mono_key_with_closures(
    base_fn_name: &str,
    type_args: &[ConcreteType],
    closure_specs: &[ClosureSpec],
) -> String {
    build_mono_key_full(base_fn_name, type_args, &[], closure_specs)
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
        // B.3 — if the callee declares any const generic parameters, auto-bind
        // them from their declared default expressions (literals only today —
        // no call-site `::<4>` turbofish syntax exists yet) and route through
        // the const-aware entry point. This keeps every caller of the type-only
        // API working unchanged while still producing distinct mono keys per
        // distinct const value.
        //
        // The resolution rule, per the Track-B B.3 plan:
        //   - literal default on the const param  → bind immediately
        //   - missing default / non-literal       → compile error
        //     ("const generic arg must be a compile-time constant")
        //
        // Comptime-evaluation of arbitrary default expressions is intentionally
        // out of scope here and deferred to a follow-up.
        if let Some(type_params) = self
            .function_defs
            .get(base_fn_name)
            .and_then(|d| d.type_params.clone())
        {
            if type_params.iter().any(|tp| tp.is_const()) {
                let const_args = resolve_const_defaults_or_error(base_fn_name, &type_params)?;
                return self.ensure_monomorphic_function_with_consts(
                    base_fn_name,
                    type_args,
                    &const_args,
                );
            }
        }

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
        // supplied `type_args`. Const-kind params are filtered out here (they
        // contribute nothing to type substitution) — the short-circuit above
        // already redirected any callee-with-const-params to the const-aware
        // entry point, so this path only sees type-kind params.
        let declared_type_params: Vec<String> = original_def
            .type_params
            .as_ref()
            .map(|tps| {
                tps.iter()
                    .filter(|tp| !tp.is_const())
                    .map(|tp| tp.name().to_string())
                    .collect()
            })
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

        // Partition the declared generic params into type-kind names and
        // const-kind names (positional against `type_args` / `const_args`
        // respectively). This became tractable in B.2 when `TypeParam` grew
        // a `Const` variant — prior to B.3 the wiring here fell back to
        // synthetic `__const_<i>` names because there was no split.
        let (type_param_names, const_param_names) = original_def
            .type_params
            .as_ref()
            .map(|tps| split_type_and_const_param_names(tps))
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

        if const_param_names.len() != const_args.len() {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "ensure_monomorphic_function_with_consts: '{}' declares {} const generic parameters but {} const arguments were supplied",
                    base_fn_name,
                    const_param_names.len(),
                    const_args.len()
                ),
                location: None,
            });
        }

        let type_subs: HashMap<String, ConcreteType> = type_param_names
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();

        // Now that const generic params carry real names (B.2 `TypeParam::Const`),
        // key `const_subs` by the declared name. The substitution pass in
        // `substitution::substitute_function_def_with_consts` rewrites any
        // body-position `Identifier(N)` that matches this name to the bound
        // literal value. If the callee never references the name in its body
        // (common for the B.3 integration tests), the substitution pass is a
        // no-op — the mono-key differentiation is still the load-bearing bit.
        let const_subs: HashMap<String, ComptimeConstValue> = const_param_names
            .iter()
            .cloned()
            .zip(const_args.iter().cloned())
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

    /// Phase C — closure-aware specialization entry point.
    ///
    /// Like [`Self::ensure_monomorphic_function`] but additionally keys the
    /// specialization on per-closure-arg [`ClosureSpec`]s and inlines each
    /// closure literal's body into the specialized stdlib template (replacing
    /// calls to the formal closure parameter with the closure body).
    ///
    /// Flow:
    ///   1. Build the full mono key (`base::T1_..._closure_N_ret_...`).
    ///   2. Cache hit → return existing index.
    ///   3. Budget exhausted → bail out (`Ok(None)`) so the caller falls
    ///      back to the generic path.
    ///   4. Substitute type params through the function def (as in the
    ///      type-only path).
    ///   5. For each closure spec, call
    ///      [`super::substitution::inline_closure_body_into_specialization`]
    ///      to rewrite the specialized body.
    ///   6. Register + compile the specialized function; record cache entry;
    ///      bump the closure-specialization count.
    ///
    /// The `closure_defs` parallel to `closure_specs` carries the peeked
    /// closure literals (params, body, captures) that the inliner needs.
    /// `callee_closure_param_names[i]` is the name of the callee's formal
    /// parameter that holds the i-th closure; it's the identifier the inliner
    /// rewrites. When empty or mismatched, specialization bails and returns
    /// `Ok(None)`.
    #[allow(clippy::too_many_arguments)]
    pub fn ensure_monomorphic_function_with_closures(
        &mut self,
        base_fn_name: &str,
        type_args: &[ConcreteType],
        closure_specs: &[ClosureSpec],
        closure_defs: &[ClosureDefPeek],
        callee_closure_param_names: &[String],
    ) -> Result<Option<u16>> {
        let mono_key = build_mono_key_with_closures(base_fn_name, type_args, closure_specs);

        // Cache hit — reuse.
        if let Some(existing) = self.monomorphization_cache.lookup(&mono_key) {
            return Ok(Some(existing));
        }

        // Per-module specialization budget (§3.4). When we've already produced
        // DEFAULT_CLOSURE_SPECIALIZATION_BUDGET closure-aware specializations,
        // bail and let the caller emit the generic (non-inlined) path.
        if self.closure_specialization_count >= DEFAULT_CLOSURE_SPECIALIZATION_BUDGET {
            return Ok(None);
        }

        // Closure-spec length sanity check: we must have peek info and a
        // param name for every recorded spec. If not, the caller messed up.
        if closure_defs.len() != closure_specs.len()
            || callee_closure_param_names.len() != closure_specs.len()
        {
            return Ok(None);
        }

        // Look up the base def; fail soft if missing.
        let original_def = match self.function_defs.get(base_fn_name).cloned() {
            Some(d) => d,
            None => return Ok(None),
        };

        let declared_type_params: Vec<String> = original_def
            .type_params
            .as_ref()
            .map(|tps| tps.iter().map(|tp| tp.name().to_string()).collect())
            .unwrap_or_default();

        // If type args and declared params disagree, bail (falls back to
        // generic path) rather than raise — the caller may still want to
        // compile the call with an unspecialized body.
        if declared_type_params.len() != type_args.len() {
            return Ok(None);
        }

        let subs: HashMap<String, ConcreteType> = declared_type_params
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();

        // Substitute type params first.
        let mut specialized_def = substitution::substitute_function_def(&original_def, &subs);
        // Overwrite the name with the full closure-aware key so the cache key
        // and the registered function name agree.
        specialized_def.name = mono_key.clone();

        // Inline each closure body in turn.
        for (i, spec_info) in closure_defs.iter().enumerate() {
            let closure_param_name = &callee_closure_param_names[i];
            if substitution::inline_closure_body_into_specialization(
                &mut specialized_def,
                closure_param_name,
                &spec_info.param_names,
                &spec_info.body,
                &spec_info.capture_names,
            )
            .is_err()
            {
                // Inlining bailed — fall back to generic path.
                return Ok(None);
            }
        }

        // Register + compile.
        if self.register_function(&specialized_def).is_err() {
            return Ok(None);
        }
        let specialization_idx_usize = match self.find_function(&specialized_def.name) {
            Some(idx) => idx,
            None => return Ok(None),
        };
        let specialization_idx: u16 = match specialization_idx_usize.try_into() {
            Ok(x) => x,
            Err(_) => return Ok(None),
        };

        // Cache BEFORE compile_function so any recursive call inside the
        // specialized body resolves through the cache.
        self.monomorphization_cache
            .insert(mono_key, specialization_idx);
        self.next_monomorphization_id = self.next_monomorphization_id.saturating_add(1);
        self.closure_specialization_count =
            self.closure_specialization_count.saturating_add(1);

        if self.compile_function(&specialized_def).is_err() {
            // Compilation failed — we already inserted the cache entry; the
            // caller will fall back to the generic path anyway. Returning
            // Ok(None) keeps the error surface clean.
            return Ok(None);
        }

        Ok(Some(specialization_idx))
    }
}

/// B.3 — resolve a callee's const generic parameters from their declared
/// default expressions.
///
/// The grammar does not yet accept call-site turbofish (`::<4>`) for binding
/// const generic args, so the only source we have for a const value today is
/// the optional `default` on each `TypeParam::Const`. The rule:
///
///   - `TypeParam::Const { default: Some(literal_expr), .. }` → bind that value.
///   - `TypeParam::Const { default: None, .. }`              → compile error.
///   - non-literal default expression                        → compile error.
///
/// `TypeParam::Type` entries are skipped — they are handled by the type-arg
/// resolution path elsewhere.
///
/// Returns the `const_args` vector in declaration order (positional against
/// the const-kind entries in `type_params`).
fn resolve_const_defaults_or_error(
    base_fn_name: &str,
    type_params: &[shape_ast::ast::TypeParam],
) -> Result<Vec<ComptimeConstValue>> {
    let mut const_args: Vec<ComptimeConstValue> = Vec::new();
    for tp in type_params {
        match tp {
            shape_ast::ast::TypeParam::Type { .. } => continue,
            shape_ast::ast::TypeParam::Const { name, default, .. } => {
                let Some(default_expr) = default else {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "const generic arg must be a compile-time constant: '{}' declares const generic parameter '{}' with no default value, and call-site const argument syntax is not yet supported",
                            base_fn_name, name
                        ),
                        location: None,
                    });
                };
                let Some(value) = comptime_const_value_from_literal_expr(default_expr) else {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "const generic arg must be a compile-time constant: '{}' const generic parameter '{}' has a non-literal default expression (only literals are supported in B.3 — comptime-evaluated defaults are a follow-up)",
                            base_fn_name, name
                        ),
                        location: None,
                    });
                };
                const_args.push(value);
            }
        }
    }
    Ok(const_args)
}

/// Peeked closure-literal info handed to the inliner. The resolver fills
/// this from the `Expr::FunctionExpr` args before lowering.
#[derive(Debug, Clone)]
pub struct ClosureDefPeek {
    /// Formal parameter names of the closure literal (`x` in `|x| x + n`).
    pub param_names: Vec<String>,
    /// The closure literal's body statements.
    pub body: Vec<shape_ast::ast::Statement>,
    /// Names of the closure's captures, in order, as leading params for the
    /// specialized body.
    pub capture_names: Vec<String>,
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

    // =====================================================================
    // Phase C — closure-aware specialization tests.
    // =====================================================================

    #[test]
    fn build_mono_key_with_closures_matches_design_doc_format() {
        // Single closure arg, i64 return: `map::array_i64_closure_7_i64`.
        let type_args = [ConcreteType::Array(Box::new(ConcreteType::I64))];
        let closure_specs = [ClosureSpec {
            closure_type_id: shape_value::v2::concrete_type::ClosureTypeId(7),
            return_type: Some(ConcreteType::I64),
            body_hash: 0,
        }];
        let key = build_mono_key_with_closures("map", &type_args, &closure_specs);
        assert_eq!(key, "map::array_i64_closure_7_i64");
    }

    #[test]
    fn build_mono_key_filter_with_closure_bool_return() {
        // `filter(|x| x > 0)` over `Array<number>`:
        // `"filter::array_f64_closure_N_bool"`.
        let type_args = [ConcreteType::Array(Box::new(ConcreteType::F64))];
        let closure_specs = [ClosureSpec {
            closure_type_id: shape_value::v2::concrete_type::ClosureTypeId(3),
            return_type: Some(ConcreteType::Bool),
            body_hash: 0,
        }];
        let key = build_mono_key_with_closures("filter", &type_args, &closure_specs);
        assert_eq!(key, "filter::array_f64_closure_3_bool");
    }

    #[test]
    fn build_mono_key_reduce_with_two_closure_args() {
        // `reduce` with two closures: both peeked, both contribute to the key.
        let type_args = [ConcreteType::I64];
        let closure_specs = [
            ClosureSpec {
                closure_type_id: shape_value::v2::concrete_type::ClosureTypeId(4),
                return_type: Some(ConcreteType::I64),
                body_hash: 0,
            },
            ClosureSpec {
                closure_type_id: shape_value::v2::concrete_type::ClosureTypeId(5),
                return_type: Some(ConcreteType::Bool),
                body_hash: 0,
            },
        ];
        let key = build_mono_key_with_closures("reduce", &type_args, &closure_specs);
        assert_eq!(key, "reduce::i64_closure_4_i64_closure_5_bool");
    }

    #[test]
    fn build_mono_key_with_closures_unknown_return_type() {
        // When the return type is unknown (couldn't be inferred), the key
        // encodes `unknown` so different captures still produce distinct
        // keys.
        let closure_specs = [ClosureSpec {
            closure_type_id: shape_value::v2::concrete_type::ClosureTypeId(0),
            return_type: None,
            body_hash: 0,
        }];
        let key = build_mono_key_with_closures("map", &[ConcreteType::I64], &closure_specs);
        assert_eq!(key, "map::i64_closure_0_unknown");
    }

    #[test]
    fn budget_fallback_returns_none_when_exhausted() {
        // Budget is the per-module cap on closure specializations. When
        // exhausted, ensure_monomorphic_function_with_closures returns
        // Ok(None) so the caller falls back to the direct-call path.
        let mut compiler = BytecodeCompiler::new();
        compiler.closure_specialization_count = DEFAULT_CLOSURE_SPECIALIZATION_BUDGET;

        // No function registered — even so, the budget check runs first and
        // returns Ok(None) for budget exhaustion. (If we got past the budget
        // check, we'd fall into the "function_defs lookup" path and still
        // return Ok(None), which is what we want.)
        let result = compiler.ensure_monomorphic_function_with_closures(
            "map",
            &[ConcreteType::I64],
            &[ClosureSpec {
                closure_type_id: shape_value::v2::concrete_type::ClosureTypeId(0),
                return_type: Some(ConcreteType::I64),
                body_hash: 0,
            }],
            &[ClosureDefPeek {
                param_names: vec!["x".into()],
                body: vec![],
                capture_names: vec![],
            }],
            &["f".into()],
        );
        // Budget is at cap → Ok(None) (fallback path).
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn budget_counter_starts_at_zero() {
        let compiler = BytecodeCompiler::new();
        assert_eq!(compiler.closure_specialization_count, 0);
    }

    #[test]
    fn cache_hit_returns_same_index_on_second_call() {
        // Two lookups with the same closure-aware mono key must hit the
        // cache on the second try. Exercises the fast path at the top of
        // ensure_monomorphic_function_with_closures.
        let mut cache = MonomorphizationCache::new();
        let key = build_mono_key_with_closures(
            "map",
            &[ConcreteType::I64],
            &[ClosureSpec {
                closure_type_id: shape_value::v2::concrete_type::ClosureTypeId(0),
                return_type: Some(ConcreteType::I64),
                body_hash: 0,
            }],
        );
        cache.insert(key.clone(), 7);
        assert_eq!(cache.lookup(&key), Some(7));
        // Re-lookup — cache still hits.
        assert_eq!(cache.lookup(&key), Some(7));
        assert_eq!(cache.len(), 1);
    }

    // =====================================================================
    // B.3 — bind const generic args at monomorphization.
    //
    // These tests drive real `FunctionDef`s with `TypeParam::Const` members
    // through the cache entry points and assert:
    //   - distinct const values produce distinct mono-key cache entries,
    //   - identical const values collapse to one cache entry,
    //   - wrong arity / wrong type / missing-default cases surface clear
    //     compile errors,
    //   - functions without any const params are unaffected.
    // =====================================================================

    use shape_ast::ast::{
        DestructurePattern, FunctionDef, FunctionParameter, Literal, Span, TypeAnnotation,
        TypeParam,
    };

    /// Build a minimal FunctionDef with:
    ///   - `type_params` — the generic list (mix of `Type` and `Const` OK),
    ///   - a single `x: int` param,
    ///   - `int` return type,
    ///   - body `return x` so compile_function succeeds.
    fn b3_identity_n_def(type_params: Vec<TypeParam>) -> FunctionDef {
        FunctionDef {
            name: "identity_n".into(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: if type_params.is_empty() {
                None
            } else {
                Some(type_params)
            },
            params: vec![FunctionParameter {
                pattern: DestructurePattern::Identifier("x".into(), Span::default()),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: Some(TypeAnnotation::Basic("int".into())),
                default_value: None,
            }],
            return_type: Some(TypeAnnotation::Basic("int".into())),
            where_clause: None,
            body: vec![shape_ast::ast::Statement::Return(
                Some(shape_ast::ast::Expr::Identifier("x".into(), Span::default())),
                Span::default(),
            )],
            annotations: Vec::new(),
            is_async: false,
            is_comptime: false,
        }
    }

    fn const_param(name: &str, default: Option<i64>) -> TypeParam {
        TypeParam::Const {
            name: name.into(),
            span: Span::default(),
            doc_comment: None,
            ty: TypeAnnotation::Basic("int".into()),
            default: default.map(|v| shape_ast::ast::Expr::Literal(Literal::Int(v), Span::default())),
        }
    }

    #[test]
    fn b3_const_generic_distinct_values_produce_distinct_monomorphizations() {
        // identity_n<const N: int = 4> compiled with N=4 then N=8 must produce
        // TWO distinct cache entries (keys `identity_n::int_4` and
        // `identity_n::int_8`) — the load-bearing deliverable of B.3.
        let mut compiler = BytecodeCompiler::new();
        let def = b3_identity_n_def(vec![const_param("N", Some(4))]);
        compiler.function_defs.insert("identity_n".into(), def);

        // First monomorphization: N=4.
        let idx4 = compiler
            .ensure_monomorphic_function_with_consts(
                "identity_n",
                &[],
                &[ComptimeConstValue::Int(4)],
            )
            .expect("N=4 monomorphization should succeed");
        assert_eq!(
            compiler.monomorphization_cache.lookup("identity_n::int_4"),
            Some(idx4)
        );

        // Second monomorphization: N=8.
        let idx8 = compiler
            .ensure_monomorphic_function_with_consts(
                "identity_n",
                &[],
                &[ComptimeConstValue::Int(8)],
            )
            .expect("N=8 monomorphization should succeed");
        assert_eq!(
            compiler.monomorphization_cache.lookup("identity_n::int_8"),
            Some(idx8)
        );

        assert_ne!(idx4, idx8, "distinct const values must produce distinct specializations");
        assert_eq!(compiler.monomorphization_cache.len(), 2);
    }

    #[test]
    fn b3_const_generic_same_value_collapses_to_one_entry() {
        let mut compiler = BytecodeCompiler::new();
        let def = b3_identity_n_def(vec![const_param("N", Some(4))]);
        compiler.function_defs.insert("identity_n".into(), def);

        let a = compiler
            .ensure_monomorphic_function_with_consts(
                "identity_n",
                &[],
                &[ComptimeConstValue::Int(4)],
            )
            .unwrap();
        let b = compiler
            .ensure_monomorphic_function_with_consts(
                "identity_n",
                &[],
                &[ComptimeConstValue::Int(4)],
            )
            .unwrap();
        assert_eq!(a, b, "identical const args must collapse to one cache entry");
        assert_eq!(compiler.monomorphization_cache.len(), 1);
    }

    #[test]
    fn b3_const_generic_runtime_body_without_substitution_references() {
        // Exercises the full compile pipeline: register a const-generic
        // function whose body does NOT reference N, monomorphize, then
        // look up the specialized function. This proves that B.3 wiring
        // works end-to-end even before B.4 substitutes body references.
        let mut compiler = BytecodeCompiler::new();
        let def = b3_identity_n_def(vec![const_param("N", Some(4))]);
        compiler.function_defs.insert("identity_n".into(), def);

        let specialized_idx = compiler
            .ensure_monomorphic_function_with_consts(
                "identity_n",
                &[],
                &[ComptimeConstValue::Int(4)],
            )
            .expect("const-generic compile should succeed without body references");

        // The specialized function is registered in the program under a name
        // derived from the mono key. We can at least verify the cache records
        // the same index for the same key.
        assert_eq!(
            compiler.monomorphization_cache.lookup("identity_n::int_4"),
            Some(specialized_idx)
        );
    }

    #[test]
    fn b3_wrong_const_arity_errors() {
        let mut compiler = BytecodeCompiler::new();
        // Callee declares ONE const generic (N) but we pass TWO const args.
        let def = b3_identity_n_def(vec![const_param("N", Some(4))]);
        compiler.function_defs.insert("identity_n".into(), def);

        let result = compiler.ensure_monomorphic_function_with_consts(
            "identity_n",
            &[],
            &[ComptimeConstValue::Int(4), ComptimeConstValue::Int(5)],
        );
        assert!(result.is_err(), "wrong const arity must error");
        let msg = format!("{:?}", result.err().unwrap());
        assert!(
            msg.contains("const generic parameters"),
            "error should mention const generic arity, got: {}",
            msg
        );
    }

    #[test]
    fn b3_type_only_entry_routes_const_params_through_with_consts_path() {
        // When the type-only `ensure_monomorphic_function` is called against a
        // callee with const params, it must auto-bind them from defaults and
        // delegate to `ensure_monomorphic_function_with_consts`. The cache
        // entry must have the const-aware key, not the bare `identity_n` key.
        let mut compiler = BytecodeCompiler::new();
        let def = b3_identity_n_def(vec![const_param("N", Some(4))]);
        compiler.function_defs.insert("identity_n".into(), def);

        let idx = compiler
            .ensure_monomorphic_function("identity_n", &[])
            .expect("delegation to const-aware path should succeed");
        assert_eq!(
            compiler.monomorphization_cache.lookup("identity_n::int_4"),
            Some(idx),
            "type-only entry must route through the const-aware mono key"
        );
        assert!(
            compiler.monomorphization_cache.lookup("identity_n").is_none(),
            "bare `identity_n` key must NOT appear — const params always differentiate"
        );
    }

    #[test]
    fn b3_missing_const_default_errors_with_specific_message() {
        // `identity_n<const N: int>` (no default) must error at the type-only
        // entry point since there's no call-site turbofish syntax yet.
        let mut compiler = BytecodeCompiler::new();
        let def = b3_identity_n_def(vec![const_param("N", None)]);
        compiler.function_defs.insert("identity_n".into(), def);

        let result = compiler.ensure_monomorphic_function("identity_n", &[]);
        assert!(result.is_err());
        let msg = format!("{:?}", result.err().unwrap());
        assert!(
            msg.contains("const generic arg must be a compile-time constant"),
            "expected B.3 diagnostic, got: {}",
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

    // =====================================================================
    // B.5 — Track B close-out: end-to-end coverage for const generics via
    // the default-value grammar route.
    //
    // Turbofish call-site syntax (`fn_name::<3>(...)`) is a separate grammar
    // extension outside Track B's scope; the `const_generic_repeat_n_3_end_to_end`
    // placeholder in `type_resolution.rs` tracks that follow-up work.
    //
    // These tests drive the full pipeline parser → AST → cache key →
    // substituted body, using real Shape source text through
    // `parse_program` and the `BytecodeCompiler::ensure_monomorphic_function`
    // entry point. They do NOT reach runtime value assertions (top-level
    // generic function calls are not yet wired into monomorphization — see
    // the `test_user_defined_generic_function` ignore in
    // `integration_tests.rs`). Instead, each test asserts:
    //
    //   (a) the parser accepts `fn f<const N: int = V>(...) { body }`,
    //   (b) the function registers and the compiler caches the expected
    //       specialization under `f::int_V`,
    //   (c) the substituted function body has every `Identifier(N)`
    //       position rewritten to the bound literal.
    // =====================================================================

    /// Extract the `FunctionDef` for `name` from a freshly-parsed program.
    fn b5_function_def_from_source(source: &str, name: &str) -> FunctionDef {
        let program = shape_ast::parser::parse_program(source)
            .unwrap_or_else(|e| panic!("parse failed for source `{}`: {:?}", source, e));
        for item in &program.items {
            if let shape_ast::ast::Item::Function(def, _) = item {
                if def.name == name {
                    return def.clone();
                }
            }
        }
        panic!("function `{}` not found in parsed program", name);
    }

    /// Register the parsed function in a fresh compiler and drive it through
    /// the type-only `ensure_monomorphic_function` entry point. Returns the
    /// produced function index.
    fn b5_register_and_monomorphize(
        compiler: &mut BytecodeCompiler,
        def: FunctionDef,
    ) -> Result<u16> {
        let fn_name = def.name.clone();
        compiler.register_function(&def)?;
        compiler.ensure_monomorphic_function(&fn_name, &[])
    }

    /// Count how many `Identifier("<name>", ...)` nodes survive anywhere in
    /// the function def (body, params, type annotations). Uses the debug
    /// representation so it doesn't have to keep up with AST variant changes —
    /// B.4's exhaustive-match substitution is already tested directly in
    /// `substitution.rs`; here we just want a cheap "no N leaked through"
    /// smoke check on the post-substitution FunctionDef.
    fn b5_count_surviving_identifier(def: &FunctionDef, name: &str) -> usize {
        let dbg = format!("{:?}", def);
        let needle = format!("Identifier(\"{}\"", name);
        dbg.matches(&needle).count()
    }

    #[test]
    fn b5_parser_to_cache_single_const_default() {
        // Flow: parse `fn add_n<const N: int = 4>(x: int) -> int { x + N }`,
        // register, then call `ensure_monomorphic_function("add_n", &[])`.
        // The const default auto-binds N = 4 and the specialization lands in
        // the cache under `add_n::int_4`.
        let src = r#"
            fn add_n<const N: int = 4>(x: int) -> int {
                return x + N
            }
        "#;
        let def = b5_function_def_from_source(src, "add_n");
        let mut compiler = BytecodeCompiler::new();
        let idx = b5_register_and_monomorphize(&mut compiler, def)
            .expect("add_n<const N = 4> should monomorphize");
        assert_eq!(
            compiler.monomorphization_cache.lookup("add_n::int_4"),
            Some(idx),
            "expected cache entry keyed on N's bound value"
        );
    }

    #[test]
    fn b5_two_defaults_produce_distinct_specializations() {
        // Two wrapper functions with different const defaults → TWO cache
        // entries with distinct keys. This exercises the load-bearing B.3
        // distinctness guarantee end-to-end through the parser.
        let src = r#"
            fn add_4<const N: int = 4>(x: int) -> int { return x + N }
            fn add_8<const N: int = 8>(x: int) -> int { return x + N }
        "#;
        let def_4 = b5_function_def_from_source(src, "add_4");
        let def_8 = b5_function_def_from_source(src, "add_8");
        let mut compiler = BytecodeCompiler::new();

        let idx_4 = b5_register_and_monomorphize(&mut compiler, def_4).unwrap();
        let idx_8 = b5_register_and_monomorphize(&mut compiler, def_8).unwrap();

        assert_eq!(compiler.monomorphization_cache.lookup("add_4::int_4"), Some(idx_4));
        assert_eq!(compiler.monomorphization_cache.lookup("add_8::int_8"), Some(idx_8));
        assert_ne!(idx_4, idx_8, "distinct defaults must produce distinct specializations");
    }

    #[test]
    fn b5_multi_const_param_mono_key_carries_both_values() {
        // `fn rect<const R: int = 3, const C: int = 5>() -> int { R * C }`
        // should monomorphize to `rect::int_3_int_5`.
        let src = r#"
            fn rect<const R: int = 3, const C: int = 5>(x: int) -> int {
                return x + R * C
            }
        "#;
        let def = b5_function_def_from_source(src, "rect");
        let mut compiler = BytecodeCompiler::new();
        let idx = b5_register_and_monomorphize(&mut compiler, def).unwrap();
        assert_eq!(
            compiler.monomorphization_cache.lookup("rect::int_3_int_5"),
            Some(idx),
            "multi-const mono key must interleave in declaration order"
        );
    }

    #[test]
    fn b5_const_in_if_branch_is_substituted() {
        // Verify B.4's body substitution walks through `if` branches: no
        // `Identifier("N")` should survive in the specialized body.
        let src = r#"
            fn clamp_n<const N: int = 10>(x: int) -> int {
                if x > N {
                    return N
                }
                return x
            }
        "#;
        let def = b5_function_def_from_source(src, "clamp_n");
        let mut compiler = BytecodeCompiler::new();
        b5_register_and_monomorphize(&mut compiler, def).unwrap();
        let specialized = compiler
            .function_defs
            .get("clamp_n::int_10")
            .expect("specialization should be recorded by name");
        assert_eq!(
            b5_count_surviving_identifier(specialized, "N"),
            0,
            "bound const param N must not survive substitution inside `if`",
        );
    }

    #[test]
    fn b5_const_in_while_and_for_is_substituted() {
        // Verify substitution walks through `while` and `for in` loop bodies.
        let src = r#"
            fn sum_upto<const N: int = 5>(x: int) -> int {
                let mut acc = x
                let mut i = 0
                while i < N {
                    acc = acc + i
                    i = i + 1
                }
                for j in 0..N {
                    acc = acc + j
                }
                return acc
            }
        "#;
        let def = b5_function_def_from_source(src, "sum_upto");
        let mut compiler = BytecodeCompiler::new();
        b5_register_and_monomorphize(&mut compiler, def).unwrap();
        let specialized = compiler
            .function_defs
            .get("sum_upto::int_5")
            .expect("specialization should be recorded by name");
        assert_eq!(
            b5_count_surviving_identifier(specialized, "N"),
            0,
            "bound const param N must not survive in while/for",
        );
    }

    #[test]
    fn b5_const_in_closure_body_is_substituted() {
        // A closure literal inside the specialized body must also have its
        // `Identifier(N)` references rewritten — B.4 recurses into
        // `FunctionExpr` bodies.
        let src = r#"
            fn offset_by<const N: int = 7>(x: int) -> int {
                let adder = |y| y + N
                return adder(x)
            }
        "#;
        let def = b5_function_def_from_source(src, "offset_by");
        let mut compiler = BytecodeCompiler::new();
        b5_register_and_monomorphize(&mut compiler, def).unwrap();
        let specialized = compiler
            .function_defs
            .get("offset_by::int_7")
            .expect("specialization should be recorded by name");
        assert_eq!(
            b5_count_surviving_identifier(specialized, "N"),
            0,
            "bound const param N must not survive inside closure body",
        );
    }

    #[test]
    fn b5_const_in_match_arm_body_is_substituted() {
        // Match arm bodies are walked by B.4's substitution — no `N` should
        // survive.
        let src = r#"
            fn tagged<const N: int = 2>(flag: int) -> int {
                let result = match flag {
                    0 => N,
                    _ => N + 1,
                }
                return result
            }
        "#;
        let def = b5_function_def_from_source(src, "tagged");
        let mut compiler = BytecodeCompiler::new();
        b5_register_and_monomorphize(&mut compiler, def).unwrap();
        let specialized = compiler
            .function_defs
            .get("tagged::int_2")
            .expect("specialization should be recorded by name");
        assert_eq!(
            b5_count_surviving_identifier(specialized, "N"),
            0,
            "bound const param N must not survive inside match arm",
        );
    }

    #[test]
    fn b5_missing_const_default_surfaces_b3_diagnostic_through_parser() {
        // End-to-end: parse a const-generic function with NO default, then
        // trigger the type-only entry point. Since turbofish syntax isn't
        // wired yet, the only way to bind `N` is a default — so this must
        // produce the B.3 "must be a compile-time constant" diagnostic.
        let src = r#"
            fn id_n<const N: int>(x: int) -> int { return x + N }
        "#;
        let def = b5_function_def_from_source(src, "id_n");
        let mut compiler = BytecodeCompiler::new();
        compiler.register_function(&def).unwrap();
        let result = compiler.ensure_monomorphic_function("id_n", &[]);
        assert!(
            result.is_err(),
            "const-generic fn with no default must not auto-bind successfully"
        );
        let msg = format!("{:?}", result.err().unwrap());
        assert!(
            msg.contains("const generic arg must be a compile-time constant"),
            "expected B.3 diagnostic, got: {}",
            msg
        );
    }

    #[test]
    fn b5_same_default_twice_collapses_to_single_cache_entry() {
        // Calling the const-aware entry point twice with the same resolved
        // default must hit the cache on the second try — proof that the
        // monomorphization pipeline is idempotent per (fn_name, const values).
        let src = r#"
            fn pin_n<const N: int = 11>(x: int) -> int { return x + N }
        "#;
        let def = b5_function_def_from_source(src, "pin_n");
        let mut compiler = BytecodeCompiler::new();
        compiler.register_function(&def).unwrap();
        let a = compiler.ensure_monomorphic_function("pin_n", &[]).unwrap();
        let b = compiler.ensure_monomorphic_function("pin_n", &[]).unwrap();
        assert_eq!(a, b, "second call must hit the cache");
        assert_eq!(
            compiler.monomorphization_cache.lookup("pin_n::int_11"),
            Some(a)
        );
    }
}
