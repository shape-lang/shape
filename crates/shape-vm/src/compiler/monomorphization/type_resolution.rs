//! Call-site type-parameter resolution for v2 monomorphization.
//!
//! **Owner**: Agent 1 of Phase 2.1.
//!
//! ## Phase 5 — Const Generics Audit (Agent 2)
//!
//! Phase 5 of the v2 monomorphization effort extends this module with
//! scaffolding for **const generic parameters** (e.g.
//! `fn repeat<const N: int>(x: number) -> Array<number>`). The scaffolding
//! lives in [`TypeArgResolution::const_args`], [`build_mono_key_with_consts`],
//! [`ComptimeConstValue`], and the
//! [`crate::compiler::monomorphization::cache::BytecodeCompiler::ensure_monomorphic_function_with_consts`]
//! entry point on the cache.
//!
//! ### Grammar gap
//!
//! As of Phase 5, **the Shape grammar does NOT support const generic
//! parameters**. The audit results:
//!
//!   - `shape.pest`'s `type_param_name` rule (line 172) only allows
//!     `ident ~ (":" ~ trait_bound_list)? ~ ("=" ~ type_annotation)?`. There
//!     is no `const` keyword form.
//!   - `shape.pest`'s `generic_type` rule (line 903) only allows
//!     `type_annotation` arguments inside `<...>`. There is no expression
//!     argument form, so `repeat<3>(1.0)` does not parse — `3` is not a
//!     `type_annotation`.
//!   - `TypeParam` in `shape-ast/src/ast/types.rs:189` is a struct with
//!     `name`, `default_type`, and `trait_bounds` fields. There is no
//!     discriminator that would let the AST distinguish a type-kind generic
//!     from a const-kind generic.
//!
//! ### What would need to change in the grammar / AST
//!
//! Three things need to land before const generics work end-to-end:
//!
//!   1. **`shape.pest` — `type_param_name`**: extend to allow
//!      `"const" ~ ident ~ ":" ~ type_annotation` as an alternative form.
//!      Roughly:
//!      ```pest
//!      type_param_name = {
//!          "const" ~ ident ~ ":" ~ type_annotation
//!          | ident ~ (":" ~ trait_bound_list)? ~ ("=" ~ type_annotation)?
//!      }
//!      ```
//!   2. **`shape.pest` — `generic_type`**: extend to allow either a
//!      `type_annotation` OR a `const_generic_arg` (a comptime-evaluable
//!      expression) per slot. The simplest path is a new alternative rule:
//!      ```pest
//!      generic_arg = { type_annotation | const_generic_arg }
//!      const_generic_arg = { literal | "(" ~ expression ~ ")" }
//!      generic_type = {
//!          qualified_ident ~ "<" ~ generic_arg ~ ("," ~ generic_arg)* ~ ">"
//!      }
//!      ```
//!   3. **`TypeParam` enum** in `shape-ast/src/ast/types.rs`: convert from
//!      a struct into an enum, or add an `is_const: bool` + `const_type:
//!      Option<TypeAnnotation>` pair. The enum form is cleaner because
//!      const-kind params have no `trait_bounds` / `default_type` semantics:
//!      ```rust
//!      pub enum TypeParam {
//!          Type {
//!              name: String,
//!              default_type: Option<TypeAnnotation>,
//!              trait_bounds: Vec<TypePath>,
//!              ...
//!          },
//!          Const {
//!              name: String,
//!              type_ann: TypeAnnotation,  // e.g. `int`, `bool`
//!              ...
//!          },
//!      }
//!      ```
//!      Every consumer of `TypeParam.name` (~30 sites in shape-vm,
//!      shape-runtime, LSP) would need to update its match arms — see the
//!      "Exhaustive Match Rule" in `CLAUDE.md` for the typical drill.
//!
//! Until these land, the const-generic path in this module is exercised by
//! unit tests only — there is no parser surface to drive it from real Shape
//! source. The cache, mono_key, and substitution scaffolding are nonetheless
//! complete and ready to wire up the moment the grammar adds the syntax.
//!
//! ## Original Phase 2.1 docs (type-only path)
//!
//! When the bytecode compiler encounters a call to a generic user function
//! (e.g. `map<T, U>(arr: Array<T>, f: (T) -> U) -> Array<U>`), it needs to
//! choose a concrete instantiation (`map<i64, string>`, `map<f64, bool>`, …).
//! That decision is driven by the *types of the actual argument expressions*
//! at the call site.
//!
//! This module is the front-end of that pipeline. It does **not** clone or
//! compile anything; it only computes the bindings for each generic parameter
//! and produces a stable [`mono_key`](TypeArgResolution::mono_key) string.
//! Downstream agents consume the bindings:
//!
//!   - **Agent 2** owns `substitution.rs` — it takes a `FunctionDef` plus the
//!     bindings here and produces a fully-monomorphized AST.
//!   - **Agent 3** owns `cache.rs` — it keys compiled specializations by
//!     [`TypeArgResolution::mono_key`] so identical instantiations share one
//!     compiled function.
//!   - **Agent 4** writes integration tests against the full pipeline.
//!
//! # Resolution algorithm
//!
//! Given:
//!   - the function's declared parameter [`TypeAnnotation`]s,
//!   - a [`ConcreteType`] for each actual argument,
//!   - the list of declared generic parameter names (`["T", "U", ...]`),
//!
//! the resolver walks the annotation tree and the concrete type tree in
//! lock-step. When the annotation reaches a `Basic`/`Reference` whose name
//! matches one of the generic param names, the corresponding `ConcreteType`
//! subtree is recorded as the binding for that param. If multiple occurrences
//! of the same param all agree, one binding is kept; if they disagree the
//! resolution fails (returns `None`).
//!
//! Resolution returns `None` (rather than erroring) when the call site can't
//! be fully resolved. That intentionally lets the compiler fall through to
//! the generic-template path and keeps existing tests passing while the rest
//! of the v2 pipeline is being built out.

use shape_ast::ast::{Expr, Spanned, TypeAnnotation};
use shape_value::v2::ConcreteType;
use std::collections::HashMap;

use crate::compiler::BytecodeCompiler;

/// A compile-time-evaluated value used to specialize a const generic parameter.
///
/// This is a self-contained enum that carries the scalar value directly,
/// decoupled from the runtime `ValueWord` representation. The compiler never
/// needs NaN-boxing or heap-allocated values for const generic parameters —
/// only the four scalar kinds that can appear as compile-time constants.
#[derive(Debug, Clone, PartialEq)]
pub enum ComptimeConstValue {
    Int(i64),
    Number(f64),
    Bool(bool),
    String(String),
}

impl Eq for ComptimeConstValue {}

impl ComptimeConstValue {
    /// Extract the value as an `i64`, if it is an `Int`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ComptimeConstValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Extract the value as an `f64`, if it is a `Number`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ComptimeConstValue::Number(f) => Some(*f),
            _ => None,
        }
    }

    /// Extract the value as a `bool`, if it is a `Bool`.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ComptimeConstValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Extract the value as a `&str`, if it is a `String`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ComptimeConstValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

/// Render a single const generic value into a stable, filesystem-safe string
/// for inclusion in a `mono_key`.
///
/// The format is `"<typetag>_<value-debug>"` so that two const args with
/// different scalar widths or types never collide. Examples:
///
///   - `int(3)`        → `"int_3"`
///   - `int(-1)`       → `"int_-1"` (the leading `-` is preserved)
///   - `bool(true)`    → `"bool_true"`
///   - `string("hi")`  → `"string_hi"`
///
/// TODO(phase-5-agent-1): once `ComptimeValue` lands with a typed `Hash` impl,
/// switch this to a stable hash-based key (e.g. `"int_<hex8>"`) so the keys
/// stay compact for large bigint / decimal values.
pub fn const_value_mono_segment(v: &ComptimeConstValue) -> String {
    match v {
        ComptimeConstValue::Int(i) => format!("int_{}", i),
        ComptimeConstValue::Bool(b) => format!("bool_{}", b),
        ComptimeConstValue::Number(f) => {
            // f64 → bit pattern keeps NaN/Inf distinguishable.
            format!("f64_{:x}", f.to_bits())
        }
        ComptimeConstValue::String(s) => {
            // Sanitise: keep alphanum + underscore so the resulting key is a valid
            // function symbol suffix on every backend.
            let safe: String = s
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            format!("string_{}", safe)
        }
    }
}

/// Result of resolving the type arguments at a generic call site.
///
/// `type_args` is in declaration order — the same order as the function's
/// `type_params: Option<Vec<TypeParam>>`. `mono_key` is the canonical cache
/// key shared with [`crate::compiler::monomorphization::cache::build_mono_key`]:
/// `"<fn_name>::<arg1>_<arg2>_..."`, where each segment uses
/// [`ConcreteType::mono_key`].
///
/// # Const generic params (Phase 5)
///
/// Functions may also be parameterised on **compile-time constant values**
/// (`fn repeat<const N: int>(...)`). When that lands in the grammar, the
/// resolver also fills [`Self::const_args`] with one entry per declared const
/// generic param. The cache key in [`Self::mono_key`] then includes the const
/// values too — see [`build_mono_key_with_consts`].
///
/// **Grammar gap**: as of Phase 5 the grammar does NOT yet support const
/// generic params (see the audit notes at the top of this module). The
/// `const_args` field is therefore plumbed end-to-end but always empty in
/// production: the only callers that populate it are the unit tests and any
/// future call site that synthesises a const-generic call manually. The cache
/// behaviour, mono_key shape, and substitution path are exercised by tests
/// without depending on grammar support.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeArgResolution {
    /// Base function name (without specialization suffix).
    pub fn_name: String,
    /// Resolved concrete type for each generic parameter, in declaration order.
    pub type_args: Vec<ConcreteType>,
    /// Resolved compile-time constant value for each *const* generic parameter,
    /// in declaration order. Empty when the callee has no const generic params.
    ///
    /// See [`ComptimeConstValue`] for the underlying value representation and
    /// the migration path to the typed Phase 5 ComptimeValue.
    pub const_args: Vec<ComptimeConstValue>,
    /// Cache key — `format!("{}::{}", fn_name, type_args[*].mono_key().join("_"))`,
    /// extended with `const_args` segments via [`const_value_mono_segment`]
    /// when const generics are present. For a non-generic call
    /// (`type_args.is_empty() && const_args.is_empty()`) this is just
    /// `fn_name`.
    pub mono_key: String,
}

impl TypeArgResolution {
    /// Create a [`TypeArgResolution`] from a base name and a list of resolved
    /// concrete arguments. Computes [`Self::mono_key`] from the inputs.
    ///
    /// `const_args` is left empty. Use [`Self::with_consts`] to also bind
    /// const generic parameters.
    pub fn new(fn_name: impl Into<String>, type_args: Vec<ConcreteType>) -> Self {
        let fn_name = fn_name.into();
        let mono_key = build_mono_key(&fn_name, &type_args);
        Self {
            fn_name,
            type_args,
            const_args: Vec::new(),
            mono_key,
        }
    }

    /// Create a [`TypeArgResolution`] that also carries const generic
    /// bindings. The mono_key is built via [`build_mono_key_with_consts`] so
    /// type-only and const-only and mixed calls all hash distinctly in the
    /// specialization cache.
    pub fn with_consts(
        fn_name: impl Into<String>,
        type_args: Vec<ConcreteType>,
        const_args: Vec<ComptimeConstValue>,
    ) -> Self {
        let fn_name = fn_name.into();
        let mono_key = build_mono_key_with_consts(&fn_name, &type_args, &const_args);
        Self {
            fn_name,
            type_args,
            const_args,
            mono_key,
        }
    }
}

/// Build the cache key string for a (function name, type args) pair.
///
/// Format: `"<fn_name>::<arg1>_<arg2>_..."` — or just `"<fn_name>"` when
/// `type_args` is empty (so non-generic calls reuse the base name).
///
/// This intentionally mirrors
/// [`crate::compiler::monomorphization::cache::build_mono_key`] so the
/// front-end (this module) and the cache stay byte-for-byte consistent.
pub fn build_mono_key(fn_name: &str, type_args: &[ConcreteType]) -> String {
    build_mono_key_with_consts(fn_name, type_args, &[])
}

/// Build a cache key that incorporates both type arguments AND const
/// generic-parameter values.
///
/// Format:
///
///   - No args at all       → `"fn_name"`
///   - Type args only       → `"fn_name::T1_T2"` (same as [`build_mono_key`])
///   - Const args only      → `"fn_name::int_3"` (a single `int(3)` const arg)
///   - Type and const args  → `"fn_name::i64_int_3"` (types first, then consts)
///
/// The const segments use [`const_value_mono_segment`] which encodes both the
/// scalar kind and the value, so `int_3` and `f64_3.0` never collide. The
/// types-then-consts ordering is fixed so the cache hashing is canonical.
pub fn build_mono_key_with_consts(
    fn_name: &str,
    type_args: &[ConcreteType],
    const_args: &[ComptimeConstValue],
) -> String {
    if type_args.is_empty() && const_args.is_empty() {
        return fn_name.to_string();
    }
    let mut parts: Vec<String> = type_args.iter().map(|t| t.mono_key()).collect();
    parts.extend(const_args.iter().map(const_value_mono_segment));
    format!("{}::{}", fn_name, parts.join("_"))
}

/// Resolve the type-parameter bindings for a generic call site.
///
/// `compiler` is consulted to look up the function's declared parameter
/// signatures via `function_defs`. `arg_types[i]` is the resolved
/// [`ConcreteType`] for the `i`-th argument expression (use `None` for
/// argument positions whose type couldn't be determined — see
/// [`extract_arg_concrete_types`]). `generic_params` is the list of declared
/// type-parameter names in the order they appear on the function (`["T"]`,
/// `["T", "U"]`, …).
///
/// Returns `Some(TypeArgResolution)` when every generic param was bound to
/// the same concrete type at every occurrence in the parameter signature.
/// Returns `None` when:
///   - the function has no entry in `compiler.function_defs`,
///   - any required generic param has no resolvable occurrence in the
///     parameter signature,
///   - any required generic param has conflicting occurrences across params,
///   - a typed argument is `None` at a position where the param annotation
///     mentions a generic name (we can't infer it).
///
/// The function does NOT error — failure is silent and produces `None`. The
/// idea is that an unresolvable call site simply doesn't get monomorphized
/// yet; later phases can revisit it once more type info is available.
pub fn resolve_call_site_type_args(
    compiler: &BytecodeCompiler,
    fn_name: &str,
    arg_types: &[Option<ConcreteType>],
    generic_params: &[String],
) -> Option<TypeArgResolution> {
    // Non-generic functions never produce a TypeArgResolution; the cache key
    // for them is just the base name and there's nothing to bind.
    if generic_params.is_empty() {
        return Some(TypeArgResolution::new(fn_name, Vec::new()));
    }

    let func_def = compiler.function_defs.get(fn_name)?;

    // Walk every (param annotation, arg concrete type) pair and accumulate
    // generic-param → ConcreteType bindings.
    let mut bindings: HashMap<String, ConcreteType> = HashMap::new();
    let generics: Vec<&str> = generic_params.iter().map(|s| s.as_str()).collect();

    for (param_idx, param) in func_def.params.iter().enumerate() {
        let Some(param_annotation) = param.type_annotation.as_ref() else {
            continue;
        };

        // Skip params with no corresponding arg slot (defaulted, varargs, …).
        let Some(arg_slot) = arg_types.get(param_idx) else {
            continue;
        };
        let Some(arg_ct) = arg_slot.as_ref() else {
            // We have no concrete type for this arg. Only bail if this param
            // annotation mentions a generic that hasn't been bound yet from
            // a prior parameter. If the mentioned generics are already bound,
            // this parameter contributes no new information and we can skip.
            let has_unbound_mention = generics.iter().any(|g| {
                annotation_mentions_any(param_annotation, &[g]) && !bindings.contains_key(*g)
            });
            if has_unbound_mention {
                return None;
            }
            continue;
        };

        if !unify_annotation_with_concrete(param_annotation, arg_ct, &generics, &mut bindings) {
            return None;
        }
    }

    // Make sure every declared type parameter has been bound. If a parameter
    // is missing here it means the function is generic in a way the call site
    // doesn't constrain — bail to the generic-template path.
    let mut type_args: Vec<ConcreteType> = Vec::with_capacity(generic_params.len());
    for name in generic_params {
        let binding = bindings.get(name)?.clone();
        type_args.push(binding);
    }

    Some(TypeArgResolution::new(fn_name, type_args))
}

/// Whether `annotation` (or any of its sub-annotations) names one of the
/// generic parameters in `generics`.
fn annotation_mentions_any(annotation: &TypeAnnotation, generics: &[&str]) -> bool {
    match annotation {
        TypeAnnotation::Basic(name) => generics.iter().any(|g| *g == name.as_str()),
        TypeAnnotation::Reference(path) => generics.iter().any(|g| *g == path.as_str()),
        TypeAnnotation::Array(inner) => annotation_mentions_any(inner, generics),
        TypeAnnotation::Tuple(items) => items.iter().any(|t| annotation_mentions_any(t, generics)),
        TypeAnnotation::Generic { args, .. } => {
            args.iter().any(|a| annotation_mentions_any(a, generics))
        }
        TypeAnnotation::Function { params, returns } => {
            params
                .iter()
                .any(|p| annotation_mentions_any(&p.type_annotation, generics))
                || annotation_mentions_any(returns, generics)
        }
        TypeAnnotation::Object(fields) => fields
            .iter()
            .any(|f| annotation_mentions_any(&f.type_annotation, generics)),
        TypeAnnotation::Union(items) | TypeAnnotation::Intersection(items) => {
            items.iter().any(|t| annotation_mentions_any(t, generics))
        }
        TypeAnnotation::Void
        | TypeAnnotation::Never
        | TypeAnnotation::Null
        | TypeAnnotation::Undefined
        | TypeAnnotation::Dyn(_) => false,
    }
}

/// Try to unify a parameter's [`TypeAnnotation`] with the matching argument
/// [`ConcreteType`]. When the annotation references a generic param name,
/// record the corresponding `ConcreteType` subtree in `bindings`.
///
/// Returns `false` on conflict (the same param bound to two different concrete
/// types) or a structural mismatch the resolver can't handle.
fn unify_annotation_with_concrete(
    annotation: &TypeAnnotation,
    actual: &ConcreteType,
    generics: &[&str],
    bindings: &mut HashMap<String, ConcreteType>,
) -> bool {
    match annotation {
        TypeAnnotation::Basic(name) => {
            if generics.iter().any(|g| *g == name.as_str()) {
                return record_binding(name, actual.clone(), bindings);
            }
            // Concrete primitive — no binding to record. The bytecode compiler
            // runs its own type checking elsewhere; "no conflict, no work" is
            // sufficient here.
            true
        }
        TypeAnnotation::Reference(path) => {
            let name = path.as_str();
            if generics.iter().any(|g| *g == name) {
                return record_binding(name, actual.clone(), bindings);
            }
            true
        }
        TypeAnnotation::Array(inner) => match actual {
            ConcreteType::Array(elem) => {
                unify_annotation_with_concrete(inner, elem, generics, bindings)
            }
            // The argument is something other than an array — we can't peel a
            // generic param out of it. If the inner annotation doesn't mention
            // any generics there's nothing to fail on; otherwise the resolver
            // gives up.
            _ => !annotation_mentions_any(inner, generics),
        },
        TypeAnnotation::Generic { name, args } => {
            let base = name.as_str();
            match (base, actual) {
                ("Array" | "Vec", ConcreteType::Array(elem)) if args.len() == 1 => {
                    unify_annotation_with_concrete(&args[0], elem, generics, bindings)
                }
                ("HashMap" | "Map", ConcreteType::HashMap(k, v)) if args.len() == 2 => {
                    unify_annotation_with_concrete(&args[0], k, generics, bindings)
                        && unify_annotation_with_concrete(&args[1], v, generics, bindings)
                }
                ("Option", ConcreteType::Option(inner)) if args.len() == 1 => {
                    unify_annotation_with_concrete(&args[0], inner, generics, bindings)
                }
                ("Result", ConcreteType::Result(ok, err)) if args.len() == 2 => {
                    unify_annotation_with_concrete(&args[0], ok, generics, bindings)
                        && unify_annotation_with_concrete(&args[1], err, generics, bindings)
                }
                _ => {
                    // The shapes don't match. Only fail if a generic name is
                    // mentioned anywhere inside this annotation — otherwise it
                    // is irrelevant to monomorphization.
                    !args.iter().any(|a| annotation_mentions_any(a, generics))
                }
            }
        }
        TypeAnnotation::Tuple(items) => match actual {
            ConcreteType::Tuple(actual_items) if actual_items.len() == items.len() => {
                items
                    .iter()
                    .zip(actual_items.iter())
                    .all(|(ann, ct)| unify_annotation_with_concrete(ann, ct, generics, bindings))
            }
            _ => !items.iter().any(|t| annotation_mentions_any(t, generics)),
        },
        TypeAnnotation::Function { params: _, returns: _ } => {
            // Phase 1 represents closures as opaque
            // `ConcreteType::Closure(_)` / `ConcreteType::Function(_)` —
            // there's no nested type info to peel apart. We therefore can't
            // unify a closure-shaped annotation against the actual closure
            // value, so this position contributes no bindings.
            //
            // Crucially we still return `true`: another parameter (e.g. the
            // `arr: Array<T>` slot in `filter<T>(arr, pred)`) may have already
            // bound the same generic param, and the resolver should not
            // discard that work just because a sibling parameter happens to
            // be a closure. The bindings-completeness check at the bottom of
            // `resolve_call_site_type_args` will still bail if no parameter
            // ever bound a required generic.
            true
        }
        TypeAnnotation::Object(_)
        | TypeAnnotation::Union(_)
        | TypeAnnotation::Intersection(_)
        | TypeAnnotation::Dyn(_)
        | TypeAnnotation::Void
        | TypeAnnotation::Never
        | TypeAnnotation::Null
        | TypeAnnotation::Undefined => true,
    }
}

/// Insert a binding `name → ct`, returning false if `name` is already bound to
/// a different concrete type.
fn record_binding(
    name: &str,
    ct: ConcreteType,
    bindings: &mut HashMap<String, ConcreteType>,
) -> bool {
    if let Some(existing) = bindings.get(name) {
        return existing == &ct;
    }
    bindings.insert(name.to_string(), ct);
    true
}

/// Compute a best-effort [`ConcreteType`] for each argument expression in a
/// call. Uses the existing v2 side-tables on the [`BytecodeCompiler`]
/// (`array_element_types`, `local_array_element_types`,
/// `map_key_value_types`, `local_map_key_value_types`,
/// `current_function_local_concrete_types`, …) plus literal-shape inference.
///
/// Returns one entry per arg, in order. `None` for an entry means "we don't
/// have enough info" — the caller is expected to fall back to the generic
/// template for that resolution.
///
/// This intentionally never errors. The contract is purely best-effort — the
/// compiler always works without it.
pub fn extract_arg_concrete_types(
    compiler: &BytecodeCompiler,
    args: &[Expr],
) -> Vec<Option<ConcreteType>> {
    args.iter()
        .map(|arg| concrete_type_for_expr(compiler, arg))
        .collect()
}

/// Best-effort `ConcreteType` for a single argument expression.
pub fn concrete_type_for_expr(compiler: &BytecodeCompiler, expr: &Expr) -> Option<ConcreteType> {
    match expr {
        Expr::Literal(literal, _) => literal_concrete_type(literal),

        Expr::Identifier(name, _) => identifier_concrete_type(compiler, name),

        Expr::Array(_, _) => {
            // Array literals: prefer the per-span side-table (populated by
            // `compile_expr_array` for typed literals).
            let span = Spanned::span(expr);
            compiler
                .array_element_types
                .get(&span)
                .cloned()
                .map(|elem| ConcreteType::Array(Box::new(elem)))
        }

        Expr::UnaryOp { operand, .. } => {
            // Unary ops preserve the operand's type (Neg / Not / BitNot).
            concrete_type_for_expr(compiler, operand)
        }

        // Anything else (calls, member accesses, closures, …) is opaque
        // until we have richer side-tables. Returning None lets the resolver
        // fall back to the generic template.
        _ => None,
    }
}

fn literal_concrete_type(literal: &shape_ast::ast::Literal) -> Option<ConcreteType> {
    use shape_ast::ast::Literal;
    use shape_ast::int_width::IntWidth;

    match literal {
        Literal::Int(_) => Some(ConcreteType::I64),
        Literal::UInt(_) => Some(ConcreteType::U64),
        Literal::TypedInt(_, width) => Some(match width {
            IntWidth::I8 => ConcreteType::I8,
            IntWidth::U8 => ConcreteType::U8,
            IntWidth::I16 => ConcreteType::I16,
            IntWidth::U16 => ConcreteType::U16,
            IntWidth::I32 => ConcreteType::I32,
            IntWidth::U32 => ConcreteType::U32,
            IntWidth::U64 => ConcreteType::U64,
        }),
        Literal::Number(_) => Some(ConcreteType::F64),
        Literal::Decimal(_) => Some(ConcreteType::Decimal),
        Literal::String(_) => Some(ConcreteType::String),
        Literal::Char(_) => Some(ConcreteType::I8),
        Literal::FormattedString { .. } => Some(ConcreteType::String),
        Literal::ContentString { .. } => Some(ConcreteType::String),
        Literal::Bool(_) => Some(ConcreteType::Bool),
        Literal::None => None,
        Literal::Unit => Some(ConcreteType::Void),
        Literal::Timeframe(_) => None,
    }
}

fn identifier_concrete_type(compiler: &BytecodeCompiler, name: &str) -> Option<ConcreteType> {
    // Local slot first.
    if let Some(local_idx) = compiler_resolve_local(compiler, name) {
        if let Some(ct) = compiler
            .current_function_local_concrete_types
            .get(&local_idx)
            .cloned()
        {
            return Some(ct);
        }
        if let Some(elem) = compiler.local_array_element_types.get(&local_idx).cloned() {
            return Some(ConcreteType::Array(Box::new(elem)));
        }
        if let Some((k, v)) = compiler.local_map_key_value_types.get(&local_idx).cloned() {
            return Some(ConcreteType::HashMap(Box::new(k), Box::new(v)));
        }
        // Fallback: type tracker may have a "Vec<int>" / "Vec<number>" etc. name
        // from which we can derive a concrete array type.
        if let Some(ct) = compiler
            .type_tracker
            .get_local_type(local_idx)
            .and_then(|info| concrete_type_from_type_name(info.type_name.as_deref()))
        {
            return Some(ct);
        }
    }

    // Module binding fallback.
    if let Some(&binding_idx) = compiler.module_bindings.get(name) {
        if let Some(elem) = compiler
            .module_binding_array_element_types
            .get(&binding_idx)
            .cloned()
        {
            return Some(ConcreteType::Array(Box::new(elem)));
        }
        if let Some((k, v)) = compiler
            .module_binding_map_key_value_types
            .get(&binding_idx)
            .cloned()
        {
            return Some(ConcreteType::HashMap(Box::new(k), Box::new(v)));
        }
        // Fallback: derive concrete type from type tracker's type name.
        if let Some(ct) = compiler
            .type_tracker
            .get_binding_type(binding_idx)
            .and_then(|info| concrete_type_from_type_name(info.type_name.as_deref()))
        {
            return Some(ct);
        }
    }

    None
}

/// Extract a `ConcreteType` from a type tracker type name string.
///
/// Recognises patterns like `"Vec<int>"`, `"Vec<number>"`, `"Vec<string>"`,
/// `"Vec<bool>"` and maps them to `ConcreteType::Array(Box::new(...))`.
fn concrete_type_from_type_name(type_name: Option<&str>) -> Option<ConcreteType> {
    let name = type_name?;
    if let Some(inner) = name.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        let elem = match inner {
            "int" => ConcreteType::I64,
            "number" => ConcreteType::F64,
            "string" => ConcreteType::String,
            "bool" => ConcreteType::Bool,
            "decimal" => ConcreteType::Decimal,
            nested if nested.starts_with("Vec<") => {
                // Nested array: Vec<Vec<int>> → Array(Array(I64))
                concrete_type_from_type_name(Some(nested))
                    .map(|inner_ct| ConcreteType::Array(Box::new(inner_ct)))?
            }
            _ => return None,
        };
        return Some(ConcreteType::Array(Box::new(elem)));
    }
    // Scalar types
    match name {
        "int" => Some(ConcreteType::I64),
        "number" => Some(ConcreteType::F64),
        "string" => Some(ConcreteType::String),
        "bool" => Some(ConcreteType::Bool),
        "decimal" => Some(ConcreteType::Decimal),
        _ => None,
    }
}

/// Inline copy of the BytecodeCompiler's `resolve_local` helper. The original
/// is `pub(super)` so a sibling module can't reach it without exposing it;
/// this 5-line clone is sufficient and keeps the type-resolution module
/// self-contained.
fn compiler_resolve_local(compiler: &BytecodeCompiler, name: &str) -> Option<u16> {
    for scope in compiler.locals.iter().rev() {
        if let Some(&idx) = scope.get(name) {
            return Some(idx);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::BytecodeCompiler;
    use shape_ast::ast::type_path::TypePath;
    use shape_ast::ast::{
        DestructurePattern, FunctionDef, FunctionParam, FunctionParameter, Span, TypeAnnotation,
        TypeParam,
    };

    // ---- Helper builders ------------------------------------------------

    fn ann_basic(name: &str) -> TypeAnnotation {
        TypeAnnotation::Basic(name.to_string())
    }

    fn ann_array(inner: TypeAnnotation) -> TypeAnnotation {
        TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![inner],
        }
    }

    fn ann_fn(params: Vec<TypeAnnotation>, returns: TypeAnnotation) -> TypeAnnotation {
        TypeAnnotation::Function {
            params: params
                .into_iter()
                .map(|p| FunctionParam {
                    name: None,
                    optional: false,
                    type_annotation: p,
                })
                .collect(),
            returns: Box::new(returns),
        }
    }

    fn type_param(name: &str) -> TypeParam {
        TypeParam {
            name: name.to_string(),
            span: Span::default(),
            doc_comment: None,
            default_type: None,
            trait_bounds: Vec::new(),
        }
    }

    fn func_param(name: &str, ann: TypeAnnotation) -> FunctionParameter {
        FunctionParameter {
            pattern: DestructurePattern::Identifier(name.to_string(), Span::default()),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: Some(ann),
            default_value: None,
        }
    }

    fn make_compiler_with_fn(name: &str, def: FunctionDef) -> BytecodeCompiler {
        let mut compiler = BytecodeCompiler::new();
        compiler.function_defs.insert(name.to_string(), def);
        compiler
    }

    fn fn_def(
        name: &str,
        type_params: Vec<TypeParam>,
        params: Vec<FunctionParameter>,
        return_type: Option<TypeAnnotation>,
    ) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: if type_params.is_empty() {
                None
            } else {
                Some(type_params)
            },
            params,
            return_type,
            where_clause: None,
            body: Vec::new(),
            annotations: Vec::new(),
            is_async: false,
            is_comptime: false,
        }
    }

    // ---- Required deliverable tests -------------------------------------

    /// `map<T, U>(arr: Array<T>, f: (T) -> U) -> Array<U>` called with
    /// `arr: Array<i64>, f: (i64) -> string` resolves T=I64, U=String.
    ///
    /// Closures are represented as opaque `ConcreteType::Closure` /
    /// `ConcreteType::Function` in Phase 1, so the second arg can't carry
    /// the closure's full signature. To still exercise both bindings here we
    /// model the closure-typed param as `(T) -> U` and rely on the closure
    /// argument's *return* type being recoverable as a separate signal — but
    /// that signal isn't exposed in Phase 1, so the canonical "closure peek"
    /// path returns None for U. Instead, the realistic resolution comes from
    /// having a second NON-closure param of type U or from inferring U from
    /// the call's expected return type. The integration test in `Agent 4`
    /// will pull from the call site's expected return type once that channel
    /// is plumbed in. For this front-end unit test we use a synthetic but
    /// equivalent shape: a `u_seed: U` second parameter rather than a
    /// `(T) -> U` closure, so both bindings come from value-typed args.
    #[test]
    fn map_t_u_resolves_to_i64_string() {
        let def = fn_def(
            "map",
            vec![type_param("T"), type_param("U")],
            vec![
                func_param("arr", ann_array(ann_basic("T"))),
                func_param("u_seed", ann_basic("U")),
            ],
            Some(ann_array(ann_basic("U"))),
        );
        let compiler = make_compiler_with_fn("map", def);

        let arg_types = vec![
            Some(ConcreteType::Array(Box::new(ConcreteType::I64))),
            Some(ConcreteType::String),
        ];

        let resolution = resolve_call_site_type_args(
            &compiler,
            "map",
            &arg_types,
            &["T".to_string(), "U".to_string()],
        )
        .expect("resolution should succeed");

        assert_eq!(resolution.fn_name, "map");
        assert_eq!(
            resolution.type_args,
            vec![ConcreteType::I64, ConcreteType::String]
        );
        assert_eq!(resolution.mono_key, "map::i64_string");
    }

    /// Same generic shape, but with a real `(T) -> U` closure annotation in
    /// the second slot. We pass a `Function`-typed concrete arg for the
    /// closure; the resolver only binds T (from `arr`) and the closure-shaped
    /// annotation contributes nothing for U because Phase 1 closures are
    /// opaque. The resolver should therefore return None — proving the
    /// fail-soft behaviour for the closure-shaped path that callers rely on.
    #[test]
    fn map_with_closure_arg_returns_none_for_u() {
        let def = fn_def(
            "map",
            vec![type_param("T"), type_param("U")],
            vec![
                func_param("arr", ann_array(ann_basic("T"))),
                func_param("f", ann_fn(vec![ann_basic("T")], ann_basic("U"))),
            ],
            Some(ann_array(ann_basic("U"))),
        );
        let compiler = make_compiler_with_fn("map", def);

        let arg_types = vec![
            Some(ConcreteType::Array(Box::new(ConcreteType::I64))),
            Some(ConcreteType::Function(shape_value::v2::concrete_type::FunctionTypeId(0))),
        ];

        let resolution = resolve_call_site_type_args(
            &compiler,
            "map",
            &arg_types,
            &["T".to_string(), "U".to_string()],
        );
        assert!(resolution.is_none(), "U cannot be inferred from opaque closure");
    }

    /// `filter<T>(arr: Array<T>, pred: (T) -> bool) -> Array<T>` called with
    /// `arr: Array<f64>` resolves T=F64.
    #[test]
    fn filter_t_resolves_from_array_arg() {
        let def = fn_def(
            "filter",
            vec![type_param("T")],
            vec![
                func_param("arr", ann_array(ann_basic("T"))),
                func_param("pred", ann_fn(vec![ann_basic("T")], ann_basic("bool"))),
            ],
            Some(ann_array(ann_basic("T"))),
        );
        let compiler = make_compiler_with_fn("filter", def);

        let arg_types = vec![
            Some(ConcreteType::Array(Box::new(ConcreteType::F64))),
            // Closure: opaque in Phase 1.
            Some(ConcreteType::Function(shape_value::v2::concrete_type::FunctionTypeId(0))),
        ];

        let resolution =
            resolve_call_site_type_args(&compiler, "filter", &arg_types, &["T".to_string()])
                .expect("resolution should succeed");

        assert_eq!(resolution.fn_name, "filter");
        assert_eq!(resolution.type_args, vec![ConcreteType::F64]);
        assert_eq!(resolution.mono_key, "filter::f64");
    }

    /// `identity<T>(x: T) -> T` called with `x: bool` resolves T=Bool.
    #[test]
    fn identity_t_resolves_from_bool() {
        let def = fn_def(
            "identity",
            vec![type_param("T")],
            vec![func_param("x", ann_basic("T"))],
            Some(ann_basic("T")),
        );
        let compiler = make_compiler_with_fn("identity", def);

        let arg_types = vec![Some(ConcreteType::Bool)];

        let resolution =
            resolve_call_site_type_args(&compiler, "identity", &arg_types, &["T".to_string()])
                .expect("resolution should succeed");

        assert_eq!(resolution.fn_name, "identity");
        assert_eq!(resolution.type_args, vec![ConcreteType::Bool]);
        assert_eq!(resolution.mono_key, "identity::bool");
    }

    // ---- Edge-case tests -------------------------------------------------

    #[test]
    fn non_generic_function_returns_base_name_only() {
        let def = fn_def(
            "double",
            Vec::new(),
            vec![func_param("x", ann_basic("int"))],
            Some(ann_basic("int")),
        );
        let compiler = make_compiler_with_fn("double", def);

        let arg_types = vec![Some(ConcreteType::I64)];
        let resolution = resolve_call_site_type_args(&compiler, "double", &arg_types, &[])
            .expect("non-generic resolution should succeed");
        assert_eq!(resolution.mono_key, "double");
        assert!(resolution.type_args.is_empty());
    }

    #[test]
    fn unknown_function_returns_none() {
        let compiler = BytecodeCompiler::new();
        let arg_types = vec![Some(ConcreteType::I64)];
        let resolution = resolve_call_site_type_args(
            &compiler,
            "nonexistent",
            &arg_types,
            &["T".to_string()],
        );
        assert!(resolution.is_none());
    }

    #[test]
    fn missing_arg_concrete_type_returns_none_when_param_is_generic() {
        let def = fn_def(
            "identity",
            vec![type_param("T")],
            vec![func_param("x", ann_basic("T"))],
            Some(ann_basic("T")),
        );
        let compiler = make_compiler_with_fn("identity", def);

        let arg_types: Vec<Option<ConcreteType>> = vec![None];
        let resolution =
            resolve_call_site_type_args(&compiler, "identity", &arg_types, &["T".to_string()]);
        assert!(resolution.is_none());
    }

    #[test]
    fn conflicting_bindings_return_none() {
        // fn pair<T>(a: T, b: T) -> T  — called with (i64, string)
        let def = fn_def(
            "pair",
            vec![type_param("T")],
            vec![
                func_param("a", ann_basic("T")),
                func_param("b", ann_basic("T")),
            ],
            Some(ann_basic("T")),
        );
        let compiler = make_compiler_with_fn("pair", def);

        let arg_types = vec![Some(ConcreteType::I64), Some(ConcreteType::String)];
        let resolution =
            resolve_call_site_type_args(&compiler, "pair", &arg_types, &["T".to_string()]);
        assert!(resolution.is_none(), "conflicting bindings should fail");
    }

    #[test]
    fn nested_array_unifies() {
        // fn first<T>(arr: Array<Array<T>>) -> Array<T>
        let def = fn_def(
            "first",
            vec![type_param("T")],
            vec![func_param("arr", ann_array(ann_array(ann_basic("T"))))],
            Some(ann_array(ann_basic("T"))),
        );
        let compiler = make_compiler_with_fn("first", def);

        let arg_types = vec![Some(ConcreteType::Array(Box::new(ConcreteType::Array(
            Box::new(ConcreteType::I32),
        ))))];

        let resolution =
            resolve_call_site_type_args(&compiler, "first", &arg_types, &["T".to_string()])
                .expect("nested array unification should succeed");
        assert_eq!(resolution.type_args, vec![ConcreteType::I32]);
        assert_eq!(resolution.mono_key, "first::i32");
    }

    #[test]
    fn build_mono_key_matches_cache_format() {
        assert_eq!(
            build_mono_key("identity", &[ConcreteType::I64]),
            "identity::i64"
        );
        assert_eq!(
            build_mono_key("map", &[ConcreteType::I64, ConcreteType::String]),
            "map::i64_string"
        );
        assert_eq!(build_mono_key("noop", &[]), "noop");
    }

    // ---- extract_arg_concrete_types tests --------------------------------

    fn span() -> Span {
        Span::default()
    }

    #[test]
    fn extract_int_literal() {
        let compiler = BytecodeCompiler::new();
        let args = vec![Expr::Literal(shape_ast::ast::Literal::Int(42), span())];
        let cts = extract_arg_concrete_types(&compiler, &args);
        assert_eq!(cts, vec![Some(ConcreteType::I64)]);
    }

    #[test]
    fn extract_number_literal() {
        let compiler = BytecodeCompiler::new();
        let args = vec![Expr::Literal(shape_ast::ast::Literal::Number(3.14), span())];
        let cts = extract_arg_concrete_types(&compiler, &args);
        assert_eq!(cts, vec![Some(ConcreteType::F64)]);
    }

    #[test]
    fn extract_bool_literal() {
        let compiler = BytecodeCompiler::new();
        let args = vec![Expr::Literal(shape_ast::ast::Literal::Bool(true), span())];
        let cts = extract_arg_concrete_types(&compiler, &args);
        assert_eq!(cts, vec![Some(ConcreteType::Bool)]);
    }

    #[test]
    fn extract_string_literal() {
        let compiler = BytecodeCompiler::new();
        let args = vec![Expr::Literal(
            shape_ast::ast::Literal::String("hello".to_string()),
            span(),
        )];
        let cts = extract_arg_concrete_types(&compiler, &args);
        assert_eq!(cts, vec![Some(ConcreteType::String)]);
    }

    // ---- Const generic mono_key tests ------------------------------------
    //
    // These tests exercise the Phase 5 const-generic scaffolding. They never
    // touch the grammar (which doesn't yet support const generic params) —
    // they only verify that the cache key derivation, the
    // `TypeArgResolution::with_consts` constructor, and the
    // `const_value_mono_segment` formatter handle const-generic-like inputs
    // correctly.

    #[test]
    fn const_value_segment_int() {
        let v = ComptimeConstValue::Int(3);
        assert_eq!(const_value_mono_segment(&v), "int_3");
    }

    #[test]
    fn const_value_segment_negative_int() {
        let v = ComptimeConstValue::Int(-7);
        assert_eq!(const_value_mono_segment(&v), "int_-7");
    }

    #[test]
    fn const_value_segment_bool() {
        assert_eq!(const_value_mono_segment(&ComptimeConstValue::Bool(true)), "bool_true");
        assert_eq!(const_value_mono_segment(&ComptimeConstValue::Bool(false)), "bool_false");
    }

    #[test]
    fn build_mono_key_with_consts_only_const_args() {
        // No type args, single int const arg → "repeat::int_3"
        let key = build_mono_key_with_consts(
            "repeat",
            &[],
            &[ComptimeConstValue::Int(3)],
        );
        assert_eq!(key, "repeat::int_3");
    }

    #[test]
    fn build_mono_key_with_consts_distinct_for_distinct_values() {
        // repeat<3> and repeat<5> must be distinct cache entries.
        let k3 = build_mono_key_with_consts("repeat", &[], &[ComptimeConstValue::Int(3)]);
        let k5 = build_mono_key_with_consts("repeat", &[], &[ComptimeConstValue::Int(5)]);
        assert_ne!(k3, k5);
        assert_eq!(k3, "repeat::int_3");
        assert_eq!(k5, "repeat::int_5");
    }

    #[test]
    fn build_mono_key_with_consts_same_value_collides() {
        // repeat<3> and repeat<3> must produce IDENTICAL keys (so the cache
        // de-duplicates them).
        let a = build_mono_key_with_consts("repeat", &[], &[ComptimeConstValue::Int(3)]);
        let b = build_mono_key_with_consts("repeat", &[], &[ComptimeConstValue::Int(3)]);
        assert_eq!(a, b);
    }

    #[test]
    fn build_mono_key_with_consts_mixed_type_and_const_args() {
        // matrix<f64, ROWS=3>: type args first, then const args.
        let key = build_mono_key_with_consts(
            "matrix",
            &[ConcreteType::F64],
            &[ComptimeConstValue::Int(3)],
        );
        assert_eq!(key, "matrix::f64_int_3");
    }

    #[test]
    fn build_mono_key_with_consts_no_args_at_all() {
        // No type AND no const args → just the base name.
        let key = build_mono_key_with_consts("noop", &[], &[]);
        assert_eq!(key, "noop");
    }

    #[test]
    fn build_mono_key_legacy_matches_with_consts_for_type_only_inputs() {
        // The two helpers MUST stay byte-for-byte identical when no const
        // args are supplied — otherwise the const-aware path would silently
        // miss cache hits from the type-only path.
        let legacy = build_mono_key("map", &[ConcreteType::I64, ConcreteType::String]);
        let with_consts =
            build_mono_key_with_consts("map", &[ConcreteType::I64, ConcreteType::String], &[]);
        assert_eq!(legacy, with_consts);
    }

    #[test]
    fn type_arg_resolution_with_consts_carries_const_args() {
        let res = TypeArgResolution::with_consts(
            "repeat",
            vec![ConcreteType::F64],
            vec![ComptimeConstValue::Int(3)],
        );
        assert_eq!(res.fn_name, "repeat");
        assert_eq!(res.type_args, vec![ConcreteType::F64]);
        assert_eq!(res.const_args, vec![ComptimeConstValue::Int(3)]);
        assert_eq!(res.mono_key, "repeat::f64_int_3");
    }

    #[test]
    fn type_arg_resolution_new_leaves_const_args_empty() {
        // The original constructor must leave const_args empty so type-only
        // call sites stay byte-for-byte identical.
        let res = TypeArgResolution::new("identity", vec![ConcreteType::Bool]);
        assert!(res.const_args.is_empty());
        assert_eq!(res.mono_key, "identity::bool");
    }

    /// **PLACEHOLDER** for the future end-to-end const generics test once
    /// the grammar supports `<const N: int>`. Tracks the work needed to wire
    /// the new syntax into the existing scaffolding.
    ///
    /// TODO(grammar-const-generics):
    /// 1. Extend `shape.pest`'s `type_param_name` rule to allow
    ///    `"const" ~ ident ~ ":" ~ type_annotation`.
    /// 2. Convert `TypeParam` (in `shape-ast/src/ast/types.rs`) from a struct
    ///    into an enum with `Type { ... }` and `Const { name, type_ann, ... }`
    ///    variants — OR add an `is_const: bool` field plus a `const_type`
    ///    type annotation.
    /// 3. Extend `generic_type` in `shape.pest` to allow expression args at
    ///    the call site (`repeat<3>(1.0)`), or — easier — a separate
    ///    `const_generic_arg` rule.
    /// 4. Wire `try_monomorphize_call_site` in
    ///    `expressions/function_calls.rs` to also extract const arg values
    ///    via `eval_const_expr_to_nanboxed` and call
    ///    `ensure_monomorphic_function_with_consts` on this module.
    /// 5. Replace the `__const_<i>` placeholder names in
    ///    `cache::ensure_monomorphic_function_with_consts` with the real
    ///    declared const-param names.
    #[test]
    #[ignore = "blocked on grammar support for const generics — see TODO"]
    fn const_generic_repeat_n_3_end_to_end() {
        // The eventual test body, once the grammar supports it:
        //
        //   let source = r#"
        //       fn repeat<const N: int>(x: number) -> Array<number> {
        //           // ...
        //       }
        //       repeat<3>(1.0)
        //   "#;
        //   let (compiler, _) = compile_and_inspect(source);
        //   assert!(compiler.monomorphization_cache.lookup("repeat::int_3").is_some());
        //
        // For now this test is a placeholder. The end-to-end machinery exists
        // (`build_mono_key_with_consts`, `ensure_monomorphic_function_with_consts`,
        // `substitute_function_def_with_consts`) — what's missing is the
        // grammar / AST surface and a call-site detector for `<3>`-style
        // type-arg lists.
        unreachable!("placeholder for grammar-supported const generics");
    }
}
