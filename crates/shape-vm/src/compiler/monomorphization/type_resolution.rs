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
//!      ```text
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

#![allow(clippy::approx_constant)] // arbitrary test floats; not math constants
use crate::compiler::{BindingConcreteFact, BindingConcreteFactSource};
use shape_ast::ast::{Expr, Span, Statement, TypeAnnotation};
use shape_runtime::type_system::Type;
use shape_value::v2::ConcreteType;
use shape_value::v2::concrete_type::ClosureTypeId;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Compute a stable, **span-insensitive** 64-bit hash of a closure body AST
/// for Phase C CSE (structural deduplication of closure specializations,
/// §3.4).
///
/// Two syntactically identical closures at different source locations must
/// produce the SAME hash — otherwise `arr.map(|x| x+1)` at two call sites
/// would mint two specializations even though their bodies are identical.
/// We strip `Span` information before hashing to achieve span-insensitive
/// structural equality.
///
/// Not cryptographic — this is a cache key, not a security boundary.
/// Collisions produce incorrect cache hits; for the small AST sizes inside
/// stdlib-bound closures the collision probability is negligible.
pub fn hash_closure_body(params: &[shape_ast::ast::FunctionParameter], body: &[Statement]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for p in params {
        // Param patterns: hash identifier names only, skip spans.
        hash_pattern(&p.pattern, &mut hasher);
    }
    for stmt in body {
        hash_stmt(stmt, &mut hasher);
    }
    hasher.finish()
}

fn hash_pattern(p: &shape_ast::ast::DestructurePattern, h: &mut impl Hasher) {
    use shape_ast::ast::DestructurePattern;
    match p {
        DestructurePattern::Identifier(name, _) => {
            0u8.hash(h);
            name.hash(h);
        }
        DestructurePattern::Array(items) => {
            1u8.hash(h);
            for it in items {
                hash_pattern(it, h);
            }
        }
        DestructurePattern::Object(fields) => {
            2u8.hash(h);
            for f in fields {
                f.key.hash(h);
                hash_pattern(&f.pattern, h);
            }
        }
        DestructurePattern::Rest(inner) => {
            3u8.hash(h);
            hash_pattern(inner, h);
        }
        DestructurePattern::Decomposition(bindings) => {
            4u8.hash(h);
            for b in bindings {
                b.name.hash(h);
            }
        }
    }
}

fn hash_stmt(s: &Statement, h: &mut impl Hasher) {
    use shape_ast::ast::{Statement, statements::ForInit};
    match s {
        Statement::Return(e, _) => {
            0u8.hash(h);
            if let Some(e) = e {
                hash_expr(e, h);
            }
        }
        Statement::Break(_) => 1u8.hash(h),
        Statement::Continue(_) => 2u8.hash(h),
        Statement::VariableDecl(decl, _) => {
            3u8.hash(h);
            hash_pattern(&decl.pattern, h);
            decl.is_mut.hash(h);
            if let Some(v) = &decl.value {
                hash_expr(v, h);
            }
        }
        Statement::Assignment(a, _) => {
            4u8.hash(h);
            hash_pattern(&a.pattern, h);
            hash_expr(&a.value, h);
        }
        Statement::Expression(e, _) => {
            5u8.hash(h);
            hash_expr(e, h);
        }
        Statement::For(fl, _) => {
            6u8.hash(h);
            match &fl.init {
                ForInit::ForIn { pattern, iter } => {
                    0u8.hash(h);
                    hash_pattern(pattern, h);
                    hash_expr(iter, h);
                }
                ForInit::ForC {
                    init,
                    condition,
                    update,
                } => {
                    1u8.hash(h);
                    hash_stmt(init, h);
                    hash_expr(condition, h);
                    hash_expr(update, h);
                }
            }
            for bs in &fl.body {
                hash_stmt(bs, h);
            }
        }
        Statement::While(wl, _) => {
            7u8.hash(h);
            hash_expr(&wl.condition, h);
            for bs in &wl.body {
                hash_stmt(bs, h);
            }
        }
        Statement::If(ifs, _) => {
            8u8.hash(h);
            hash_expr(&ifs.condition, h);
            for t in &ifs.then_body {
                hash_stmt(t, h);
            }
            if let Some(el) = &ifs.else_body {
                9u8.hash(h);
                for es in el {
                    hash_stmt(es, h);
                }
            }
        }
        // Comptime-only directives and Extend are never found inside closure
        // bodies — hash them opaquely so any structural difference is
        // detected.
        other => {
            255u8.hash(h);
            format!("{:?}", other).hash(h);
        }
    }
}

fn hash_expr(e: &Expr, h: &mut impl Hasher) {
    use shape_ast::ast::Expr;
    match e {
        Expr::Literal(lit, _) => {
            0u8.hash(h);
            // Literal's Debug is span-free.
            format!("{:?}", lit).hash(h);
        }
        Expr::Identifier(name, _) => {
            1u8.hash(h);
            name.hash(h);
        }
        Expr::BinaryOp {
            left, op, right, ..
        } => {
            2u8.hash(h);
            format!("{:?}", op).hash(h);
            hash_expr(left, h);
            hash_expr(right, h);
        }
        Expr::UnaryOp { op, operand, .. } => {
            3u8.hash(h);
            format!("{:?}", op).hash(h);
            hash_expr(operand, h);
        }
        Expr::FunctionCall {
            name,
            args,
            named_args,
            ..
        } => {
            4u8.hash(h);
            name.hash(h);
            for a in args {
                hash_expr(a, h);
            }
            for (k, v) in named_args {
                k.hash(h);
                hash_expr(v, h);
            }
        }
        Expr::MethodCall {
            receiver,
            method,
            args,
            named_args,
            optional,
            ..
        } => {
            5u8.hash(h);
            hash_expr(receiver, h);
            method.hash(h);
            optional.hash(h);
            for a in args {
                hash_expr(a, h);
            }
            for (k, v) in named_args {
                k.hash(h);
                hash_expr(v, h);
            }
        }
        Expr::PropertyAccess {
            object,
            property,
            optional,
            ..
        } => {
            6u8.hash(h);
            hash_expr(object, h);
            property.hash(h);
            optional.hash(h);
        }
        Expr::IndexAccess {
            object,
            index,
            end_index,
            ..
        } => {
            7u8.hash(h);
            hash_expr(object, h);
            hash_expr(index, h);
            if let Some(e) = end_index {
                hash_expr(e, h);
            }
        }
        Expr::Array(items, _) => {
            8u8.hash(h);
            for it in items {
                hash_expr(it, h);
            }
        }
        // Everything else falls back to Debug-rendering. Any span info in
        // the Debug output means some AST spans leak into the hash — but
        // since closure bodies in real programs have stable spans within a
        // single compilation unit (comparing two spans at different source
        // positions IS the thing this function exists to distinguish), the
        // mismatch is intentional for those shapes.
        other => {
            255u8.hash(h);
            format!("{:?}", other).hash(h);
        }
    }
}

use crate::compiler::BytecodeCompiler;

/// Phase C — closure spec recorded per closure-literal argument.
///
/// When a call site passes a closure literal (`Expr::FunctionExpr`) as an
/// argument to a generic higher-order method (`arr.map(|x| x + n)`), the
/// resolver mints a [`ClosureTypeId`] (idempotent, via the per-capture
/// signature registry) and infers the closure's return `ConcreteType` by
/// unifying the method's formal closure-parameter annotation against the
/// receiver's already-bound generic type arguments.
///
/// The pair `(ClosureTypeId, return_type)` is appended to the
/// [`TypeArgResolution::closure_specs`] list in positional order (one entry
/// per closure argument the resolver found). Both values contribute to the
/// mono key segment `closure_<N>_<ret_ty>` appended by [`build_mono_key_with_consts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureSpec {
    /// The closure's layout id — interned in
    /// [`crate::compiler::BytecodeCompiler::closure_registry`].
    pub closure_type_id: ClosureTypeId,
    /// Inferred return type. `None` when the resolver couldn't narrow it — the
    /// mono key then encodes `unknown` for this segment.
    pub return_type: Option<ConcreteType>,
    /// Phase C — 64-bit hash of the closure body AST (see §3.4 structural
    /// CSE). Two closures with identical capture signatures but DIFFERENT
    /// bodies (e.g. `|x| x + 1` and `|x| x * 2` — both capture nothing, so
    /// Phase A's registry gives them the same `ClosureTypeId`) must NOT
    /// share a specialization. Including the body hash in the key produces
    /// distinct cache entries per structurally-unique closure body.
    ///
    /// Derived from `format!("{:?}", body)` fed into `DefaultHasher`. Not
    /// cryptographic — it's a cache key, not a security boundary. Collisions
    /// are vanishingly unlikely for the AST sizes that appear in stdlib
    /// higher-order calls.
    pub body_hash: u64,
}

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

    /// Short type-tag suitable for diagnostics (e.g. "int", "number", "bool",
    /// "string"). Matches the Shape surface names for the declared const
    /// generic type.
    pub fn type_tag(&self) -> &'static str {
        match self {
            ComptimeConstValue::Int(_) => "int",
            ComptimeConstValue::Number(_) => "number",
            ComptimeConstValue::Bool(_) => "bool",
            ComptimeConstValue::String(_) => "string",
        }
    }
}

/// B.3 — try to extract a [`ComptimeConstValue`] from a literal expression.
///
/// Handles only literal forms today: `Int`, `Number`, `Bool`, `String`, and
/// `UnaryOp(Neg, Int|Number)` for negative literals. Returns `None` when the
/// expression is not a literal form — the caller should surface a compile
/// error to the user ("const generic arg must be a compile-time constant").
///
/// Full comptime-evaluation of arbitrary expressions (e.g. `<const N: int = 2
/// + 2>`) is intentionally out of scope for B.3 and deferred to a follow-up
/// commit; see the plan in `/home/dev/.claude/plans/v2-residuals-closeout.md`.
pub fn comptime_const_value_from_literal_expr(expr: &Expr) -> Option<ComptimeConstValue> {
    use shape_ast::ast::{Literal, UnaryOp};
    match expr {
        Expr::Literal(Literal::Int(i), _) => Some(ComptimeConstValue::Int(*i)),
        Expr::Literal(Literal::UInt(u), _) => {
            // A u64 only fits in our i64-shaped ComptimeConstValue::Int when it
            // is in range. Larger values fall through to the non-literal path
            // and surface as a clear compile error at the call site.
            i64::try_from(*u).ok().map(ComptimeConstValue::Int)
        }
        Expr::Literal(Literal::TypedInt(i, _), _) => Some(ComptimeConstValue::Int(*i)),
        Expr::Literal(Literal::Number(f), _) => Some(ComptimeConstValue::Number(*f)),
        Expr::Literal(Literal::Bool(b), _) => Some(ComptimeConstValue::Bool(*b)),
        Expr::Literal(Literal::String(s), _) => Some(ComptimeConstValue::String(s.clone())),
        Expr::UnaryOp {
            op: UnaryOp::Neg,
            operand,
            ..
        } => match operand.as_ref() {
            Expr::Literal(Literal::Int(i), _) => Some(ComptimeConstValue::Int(-*i)),
            Expr::Literal(Literal::Number(f), _) => Some(ComptimeConstValue::Number(-*f)),
            _ => None,
        },
        _ => None,
    }
}

/// B.3 — given a callee's declared `type_params`, classify each declaration
/// as type-kind or const-kind. Returns the two lists of names in declaration
/// order. Always preserves positional alignment: type-param positions are
/// matched against the caller's `type_args` slice, and const-param positions
/// against the caller's `const_args` slice.
///
/// The split mirrors what
/// [`crate::compiler::monomorphization::cache::BytecodeCompiler::ensure_monomorphic_function_with_consts`]
/// needs to build the `type_subs` and `const_subs` maps.
pub fn split_type_and_const_param_names(
    type_params: &[shape_ast::ast::TypeParam],
) -> (Vec<String>, Vec<String>) {
    let mut type_names: Vec<String> = Vec::new();
    let mut const_names: Vec<String> = Vec::new();
    for tp in type_params {
        if tp.is_const() {
            const_names.push(tp.name().to_string());
        } else {
            type_names.push(tp.name().to_string());
        }
    }
    (type_names, const_names)
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
/// TODO(phase-5-agent-1): once `ConstantValue` (sweep phase 4d, see
/// `compiler::comptime_concrete`) is wired into the comptime mini-VM and
/// gains a typed `Hash` impl, switch this to a stable hash-based key
/// (e.g. `"int_<hex8>"`) so the keys stay compact for large bigint /
/// decimal values.
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
    /// the migration path to the typed sweep-phase-4d
    /// [`crate::compiler::comptime_concrete::ConstantValue`].
    pub const_args: Vec<ComptimeConstValue>,
    /// Phase C — one entry per closure-literal argument at the call site, in
    /// positional order. Empty when no `Expr::FunctionExpr` argument was
    /// found. Each entry contributes a `closure_<N>_<ret_ty>` segment to the
    /// mono key.
    pub closure_specs: Vec<ClosureSpec>,
    /// Cache key — `format!("{}::{}", fn_name, type_args[*].mono_key().join("_"))`,
    /// extended with `const_args` segments via [`const_value_mono_segment`]
    /// when const generics are present, then with `closure_<N>_<ret_ty>`
    /// segments for each entry in `closure_specs`. For a non-generic call
    /// (`type_args.is_empty() && const_args.is_empty() && closure_specs.is_empty()`)
    /// this is just `fn_name`.
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
            closure_specs: Vec::new(),
            mono_key,
        }
    }

    /// Phase C — construct a resolution with type args and closure specs.
    ///
    /// Leaves `const_args` empty. The mono key is built via
    /// [`build_mono_key_full`] so the `closure_<N>_<ret_ty>` segments are
    /// appended after the type args.
    pub fn with_closure_specs(
        fn_name: impl Into<String>,
        type_args: Vec<ConcreteType>,
        closure_specs: Vec<ClosureSpec>,
    ) -> Self {
        let fn_name = fn_name.into();
        let mono_key = build_mono_key_full(&fn_name, &type_args, &[], &closure_specs);
        Self {
            fn_name,
            type_args,
            const_args: Vec::new(),
            closure_specs,
            mono_key,
        }
    }

    /// Create a [`TypeArgResolution`] that also carries const generic
    /// bindings. The mono_key is built via [`build_mono_key_with_consts`] so
    /// type-only and const-only and mixed calls all hash distinctly in the
    /// specialization cache.
    #[allow(dead_code)]
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
            closure_specs: Vec::new(),
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
    build_mono_key_full(fn_name, type_args, const_args, &[])
}

/// Phase C — build a cache key that incorporates type args, const args, AND
/// per-closure-arg specialization segments.
///
/// Format (types first, then consts, then closures):
///
///   - `"fn_name"` — no args of any kind.
///   - `"fn_name::T1_T2"` — type args only.
///   - `"fn_name::T1_int_3"` — mixed type + const args.
///   - `"fn_name::T1_closure_7_i64"` — type arg + one closure segment.
///   - `"fn_name::T1_closure_7_unknown"` — closure with unresolved return type.
///
/// Each closure segment is `closure_<N>_<ret_ty>` where `N` is the
/// `ClosureTypeId` (layout id) and `<ret_ty>` is the closure's inferred return
/// type rendered via [`ConcreteType::mono_key`], or `"unknown"` when the
/// return type couldn't be narrowed.
pub fn build_mono_key_full(
    fn_name: &str,
    type_args: &[ConcreteType],
    const_args: &[ComptimeConstValue],
    closure_specs: &[ClosureSpec],
) -> String {
    if type_args.is_empty() && const_args.is_empty() && closure_specs.is_empty() {
        return fn_name.to_string();
    }
    let mut parts: Vec<String> = type_args.iter().map(|t| t.mono_key()).collect();
    parts.extend(const_args.iter().map(const_value_mono_segment));
    for spec in closure_specs {
        let ret = spec
            .return_type
            .as_ref()
            .map(|t| t.mono_key())
            .unwrap_or_else(|| "unknown".to_string());
        // `body_hash` renders as `b<hex>` so the key stays filesystem-safe.
        // A body hash of 0 means "no hash computed" — in that case we skip
        // the segment so the key stays byte-for-byte identical to the
        // pre-hash variant (back-compat for unit tests that don't populate
        // body_hash).
        if spec.body_hash != 0 {
            parts.push(format!(
                "closure_{}_{}_b{:x}",
                spec.closure_type_id.0, ret, spec.body_hash
            ));
        } else {
            parts.push(format!("closure_{}_{}", spec.closure_type_id.0, ret));
        }
    }
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

/// Phase C — resolve type args AND per-closure-arg specialization info.
///
/// Like [`resolve_call_site_type_args`], but additionally inspects each
/// argument position. When an argument is an `Expr::FunctionExpr` (closure
/// literal) AND the callee's formal annotation at that position is a
/// `TypeAnnotation::Function`, the resolver:
///
///   1. Peeks the closure literal — resolves capture names → `ConcreteType`
///      via [`concrete_type_for_expr`] (captures that can't be resolved fall
///      back to `Pointer(Void)`).
///   2. Mints/looks up a [`ClosureTypeId`] via `compiler.mint_closure_type_id_peek`
///      (idempotent at the layout level — no side-effects on `closure_type_ids`).
///   3. Unifies the closure's annotated return type against already-bound
///      generics to infer `return_type`.
///
/// The emitted `ClosureSpec`s are recorded on the returned
/// `TypeArgResolution`, so the mono key ends in one `closure_<N>_<ret>`
/// segment per closure arg.
///
/// Returns `None` for the same reasons as the type-only path, with one extra:
/// if a closure arg exists but the type-arg resolver fails to bind its generic
/// params, this helper also bails (the call site simply doesn't specialize —
/// the caller then falls back to the generic dispatch path).
pub fn resolve_call_site_type_args_with_closures(
    compiler: &mut BytecodeCompiler,
    fn_name: &str,
    args: &[Expr],
    arg_types: &[Option<ConcreteType>],
    generic_params: &[String],
) -> Option<TypeArgResolution> {
    // Two-phase resolution:
    //
    //   Phase A — try the type-only resolver. It binds generics that appear
    //   in non-closure parameter annotations (the `arr: Array<T>` slot in
    //   `Vec.map<U>(f: (T) => U) -> Vec<U>` binds T from `xs: Array<Int64>`).
    //   The resolver bails on generics that ONLY appear in closure-position
    //   annotations (`U` mentioned only inside `f: (T) => U`).
    //
    //   Phase B — V3-S6a resolver extension. When Phase A bails, run a
    //   permissive variant that allows generics to remain unbound iff they
    //   appear only in closure-position annotations. Then for each closure
    //   argument, do lightweight closure-body return-type inference and
    //   unify against the callee's closure-param return annotation to bind
    //   those remaining generics.
    //
    // Both phases produce identical bindings when Phase A succeeds — the
    // permissive base resolver is a strict superset of the type-only
    // resolver's binding set.
    let resolution_args =
        match resolve_call_site_type_args(compiler, fn_name, arg_types, generic_params) {
            Some(base) => base.type_args,
            None => {
                // Phase A bailed. Try Phase B: permissive resolution that lets
                // closure-only generics remain unbound until closure-return
                // inference fills them in.
                resolve_with_closure_return_inference(
                    compiler,
                    fn_name,
                    args,
                    arg_types,
                    generic_params,
                )?
            }
        };

    // Clone the fn def (we need to hold both a &mut compiler and an immutable
    // view of param annotations). The def is not hot — one clone per closure
    // call site is negligible.
    let func_def = compiler.function_defs.get(fn_name).cloned()?;

    // Build a bindings map from the already-resolved type args, so the
    // closure-return-type inference can substitute through the closure's
    // annotation.
    let mut bindings: HashMap<String, ConcreteType> = HashMap::new();
    for (name, ct) in generic_params.iter().zip(resolution_args.iter()) {
        bindings.insert(name.clone(), ct.clone());
    }

    let mut closure_specs: Vec<ClosureSpec> = Vec::new();

    // Walk each argument position. `args` is the actual call-site expression
    // list; `func_def.params` is the callee's formal parameter list. The two
    // are aligned positionally up to `args.len()`.
    for (i, arg_expr) in args.iter().enumerate() {
        // Only closures contribute a ClosureSpec — everything else is already
        // represented in `type_args`.
        let (cparams, cbody) = match arg_expr {
            Expr::FunctionExpr { params, body, .. } => (params, body),
            _ => continue,
        };

        // The callee must have a corresponding parameter with a function-
        // shaped annotation for us to infer the return type. If it doesn't,
        // we still mint a ClosureTypeId (with `return_type = None`) so the
        // mono key is distinct per capture signature.
        let return_type = func_def
            .params
            .get(i)
            .and_then(|p| p.type_annotation.as_ref())
            .and_then(|ann| match ann {
                TypeAnnotation::Function { returns, .. } => Some(returns.as_ref()),
                _ => None,
            })
            .and_then(|ret_ann| {
                // Substitute type params through the return annotation,
                // then render to ConcreteType if possible.
                concrete_type_from_annotation(ret_ann, &bindings)
            });

        // Mint a ClosureTypeId for the literal. Uses the captures-only
        // signature (Phase A semantics) so two structurally identical closure
        // literals with identical captures share one id.
        let closure_type_id = compiler.mint_closure_type_id_peek(cparams, cbody);
        // Phase C §3.4 — structural CSE. The body hash distinguishes two
        // closures with identical capture signatures (and hence identical
        // ClosureTypeIds) but different bodies. Without this, `|x| x + 1`
        // and `|x| x * 2` would erroneously share a specialization.
        let body_hash = hash_closure_body(cparams, cbody);

        closure_specs.push(ClosureSpec {
            closure_type_id,
            return_type,
            body_hash,
        });
    }

    if closure_specs.is_empty() {
        // No closure args — return the type-only resolution unchanged so the
        // cache key stays byte-for-byte consistent with prior phases.
        return Some(TypeArgResolution::new(fn_name, resolution_args));
    }

    Some(TypeArgResolution::with_closure_specs(
        fn_name,
        resolution_args,
        closure_specs,
    ))
}

/// V3-S6a resolver extension (Phase B of [`resolve_call_site_type_args_with_closures`]).
///
/// Permissive resolution path that binds closure-return-typed generics. Runs
/// when the strict type-only resolver bails because a generic appears in a
/// closure-parameter annotation but doesn't appear in any non-closure
/// parameter (and so can't be bound from non-closure arg types alone).
///
/// Algorithm:
///
/// 1. Walk every `(param_annotation, arg_concrete_type)` pair, just like the
///    strict resolver. Unification is identical EXCEPT: when a param's
///    annotation is `TypeAnnotation::Function` AND the corresponding
///    arg-type is None (closure literal), we don't bail on unbound
///    generics — we mark them as "closure-bound, pending".
/// 2. For each closure arg, infer the closure body's return-type name via
///    `infer_closure_body_return_type_name`. Map the inferred name to a
///    `ConcreteType` and unify against the callee's closure-param return
///    annotation `(T) => U` — this binds U (and any further nested generics
///    in the closure return position).
/// 3. Bindings must be complete: every declared generic param has a
///    `ConcreteType`. If any remain unbound after closure-return inference,
///    bail.
///
/// Returns `None` when binding is incomplete OR when this call site doesn't
/// fit the closure-return-typed-generic shape (caller falls back to the
/// generic-template path).
fn resolve_with_closure_return_inference(
    compiler: &mut BytecodeCompiler,
    fn_name: &str,
    args: &[Expr],
    arg_types: &[Option<ConcreteType>],
    generic_params: &[String],
) -> Option<Vec<ConcreteType>> {
    if generic_params.is_empty() {
        // Defensive: the caller already handled this in
        // `resolve_call_site_type_args`. If we land here with no generics
        // there's nothing to do.
        return Some(Vec::new());
    }

    let func_def = compiler.function_defs.get(fn_name).cloned()?;
    let mut bindings: HashMap<String, ConcreteType> = HashMap::new();
    let generics: Vec<&str> = generic_params.iter().map(|s| s.as_str()).collect();

    // Step 1: walk non-closure params, accumulate bindings the strict
    // resolver would have accumulated.
    for (param_idx, param) in func_def.params.iter().enumerate() {
        let Some(param_annotation) = param.type_annotation.as_ref() else {
            continue;
        };

        let Some(arg_slot) = arg_types.get(param_idx) else {
            continue;
        };
        let Some(arg_ct) = arg_slot.as_ref() else {
            // No concrete type for this arg. Bail if this param's
            // annotation mentions an unbound generic AND the annotation is
            // not a closure-position shape (closure-position unbound
            // generics get bound by Step 2 below).
            let mentions_unbound_outside_closure = generics.iter().any(|g| {
                annotation_mentions_outside_closure_position(param_annotation, g)
                    && !bindings.contains_key(*g)
            });
            if mentions_unbound_outside_closure {
                return None;
            }
            continue;
        };

        if !unify_annotation_with_concrete(param_annotation, arg_ct, &generics, &mut bindings) {
            return None;
        }
    }

    // Step 2: closure-return inference. For each closure arg, infer the
    // return-type name and unify against the callee's closure-param return
    // annotation to bind closure-only generics.
    for (param_idx, arg_expr) in args.iter().enumerate() {
        let Expr::FunctionExpr {
            params: cparams,
            body: cbody,
            ..
        } = arg_expr
        else {
            continue;
        };

        // Skip if the callee has no param at this index OR no annotation
        // OR the annotation isn't a function shape.
        let Some(param) = func_def.params.get(param_idx) else {
            continue;
        };
        let Some(TypeAnnotation::Function {
            params: cparam_anns,
            returns: ret_ann,
        }) = param.type_annotation.as_ref()
        else {
            continue;
        };

        // Short-circuit: if the closure-param's return annotation doesn't
        // mention any generic, there's nothing to bind here.
        if !generics
            .iter()
            .any(|g| annotation_mentions_any(ret_ann, &[g]))
        {
            continue;
        }

        // LANG-9-spin-1-identity-closure (ADR-006 §2.7.5 producer-side stamp):
        // compute per-closure-param expected type names by substituting the
        // already-bound generics into the callee's closure-param annotations.
        // Pass to `infer_closure_body_return_type_name_with_caller_context`
        // so the closure body's bare-`Expr::Identifier(param)` terminal
        // expression resolves via `param_types[name]` seeded from the
        // caller-side substitution.
        //
        // The substituted callee-annotation IS the proof of the closure's
        // param types at this call site — no runtime probe, no fabrication.
        // Mirrors the value-call-site caller-context shape at
        // `expressions/function_calls.rs:694-705`.
        let caller_arg_type_names: Vec<Option<String>> = cparam_anns
            .iter()
            .map(|cp_ann| {
                concrete_type_from_annotation(&cp_ann.type_annotation, &bindings).and_then(|ct| {
                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(&ct)
                        .and_then(|ann| {
                            crate::compiler::BytecodeCompiler::tracked_type_name_from_annotation(
                                &ann,
                            )
                        })
                })
            })
            .collect();

        // Lightweight closure body return-type inference. The residual generic
        // resolver ABI still wants a display name, so use the name-rendering
        // wrapper over the structural engine-span lookup.
        let Some(return_type_name) =
            crate::compiler::expressions::closures::infer_closure_body_return_type_name_with_caller_context(
                compiler,
                cparams,
                cbody,
                None,
                &[],
                &caller_arg_type_names,
            )
        else {
            // Can't infer the closure's return type. Without it we can't
            // bind closure-only generics; bail.
            return None;
        };

        let Some(return_ct) = concrete_type_from_name(&return_type_name) else {
            return None;
        };

        // Unify the closure-param return annotation against the inferred
        // closure return ConcreteType to bind generics mentioned in the
        // return position.
        if !unify_annotation_with_concrete(ret_ann, &return_ct, &generics, &mut bindings) {
            return None;
        }
    }

    // Step 3: completeness check. Every declared generic must be bound.
    let mut type_args: Vec<ConcreteType> = Vec::with_capacity(generic_params.len());
    for name in generic_params {
        let binding = bindings.get(name)?.clone();
        type_args.push(binding);
    }

    Some(type_args)
}

/// V3-S6a resolver extension helper: returns true iff `annotation` mentions
/// `generic` *somewhere other than* inside a `TypeAnnotation::Function`
/// position. Used by `resolve_with_closure_return_inference` to detect
/// generics that the type-only resolver could have bound from a non-closure
/// arg — versus generics that ONLY appear in closure-param positions and
/// must wait for closure-return inference.
fn annotation_mentions_outside_closure_position(
    annotation: &TypeAnnotation,
    generic: &str,
) -> bool {
    match annotation {
        TypeAnnotation::Basic(name) => name.as_str() == generic,
        TypeAnnotation::Reference(path) => path.as_str() == generic,
        TypeAnnotation::Borrow { inner, .. } => {
            annotation_mentions_outside_closure_position(inner, generic)
        }
        TypeAnnotation::Array(inner) => {
            annotation_mentions_outside_closure_position(inner, generic)
        }
        TypeAnnotation::Tuple(items) => items
            .iter()
            .any(|t| annotation_mentions_outside_closure_position(t, generic)),
        TypeAnnotation::Generic { args, .. } => args
            .iter()
            .any(|a| annotation_mentions_outside_closure_position(a, generic)),
        // The whole point of this helper: a Function annotation is a
        // closure position, so mentions inside it are NOT outside-closure
        // mentions. Skip recursion.
        TypeAnnotation::Function { .. } => false,
        TypeAnnotation::Object(fields) => fields
            .iter()
            .any(|f| annotation_mentions_outside_closure_position(&f.type_annotation, generic)),
        TypeAnnotation::Union(items) | TypeAnnotation::Intersection(items) => items
            .iter()
            .any(|t| annotation_mentions_outside_closure_position(t, generic)),
        TypeAnnotation::Void
        | TypeAnnotation::Never
        | TypeAnnotation::Null
        | TypeAnnotation::Undefined
        | TypeAnnotation::Dyn(_) => false,
    }
}

/// Try to render a `TypeAnnotation` as a `ConcreteType`, substituting any
/// type-parameter references from `bindings`. Returns `None` when the
/// annotation mentions something the resolver can't map to a concrete type.
fn concrete_type_from_annotation(
    ann: &TypeAnnotation,
    bindings: &HashMap<String, ConcreteType>,
) -> Option<ConcreteType> {
    match ann {
        TypeAnnotation::Basic(name) => {
            if let Some(ct) = bindings.get(name) {
                return Some(ct.clone());
            }
            concrete_type_from_name(name)
        }
        TypeAnnotation::Reference(path) if !path.is_qualified() => {
            let n = path.as_str();
            if let Some(ct) = bindings.get(n) {
                return Some(ct.clone());
            }
            concrete_type_from_name(n)
        }
        TypeAnnotation::Generic { name, args } => {
            let base = name.as_str();
            match base {
                "Array" | "Vec" if args.len() == 1 => Some(ConcreteType::Array(Box::new(
                    concrete_type_from_annotation(&args[0], bindings)?,
                ))),
                "HashMap" | "Map" if args.len() == 2 => Some(ConcreteType::HashMap(
                    Box::new(concrete_type_from_annotation(&args[0], bindings)?),
                    Box::new(concrete_type_from_annotation(&args[1], bindings)?),
                )),
                "Option" if args.len() == 1 => Some(ConcreteType::Option(Box::new(
                    concrete_type_from_annotation(&args[0], bindings)?,
                ))),
                "Result" if args.len() == 2 => Some(ConcreteType::Result(
                    Box::new(concrete_type_from_annotation(&args[0], bindings)?),
                    Box::new(concrete_type_from_annotation(&args[1], bindings)?),
                )),
                // T1 sub-case (b) (strict-flip, 2026-06-20): the `Result<T>`
                // shorthand (implicit `AnyError`) is a single-arg `Result`.
                // Without this arm a fn `-> Result<int>` resolved to `None`, so
                // a `let v = (find_user(id) !! "ctx")?` binding (and its `v + 1`
                // downstream) erased to `unknown`. The error type is the implicit
                // AnyError; `?` discards it and the `!!` arm already maps it, so a
                // `Void` err placeholder is sound (mirrors the `!!`/Option arms).
                "Result" if args.len() == 1 => Some(ConcreteType::Result(
                    Box::new(concrete_type_from_annotation(&args[0], bindings)?),
                    Box::new(ConcreteType::Void),
                )),
                _ => None,
            }
        }
        TypeAnnotation::Array(inner) => Some(ConcreteType::Array(Box::new(
            concrete_type_from_annotation(inner, bindings)?,
        ))),
        TypeAnnotation::Tuple(items) => {
            let elems: Option<Vec<ConcreteType>> = items
                .iter()
                .map(|t| concrete_type_from_annotation(t, bindings))
                .collect();
            Some(ConcreteType::Tuple(elems?))
        }
        TypeAnnotation::Void => Some(ConcreteType::Void),
        _ => None,
    }
}

/// v0.3 WS-6 — compiler-aware `ConcreteType` for a declared
/// [`TypeAnnotation`], used to record the concrete type of a `let`-binding
/// with an explicit annotation so a later generic call site
/// (`let n: Option<int> = ...; id(n)`) can resolve its argument's type.
///
/// This is `concrete_type_from_annotation` extended with user-type
/// awareness: a bare `Reference`/`Basic` name that is not a primitive but
/// IS a registered struct / enum resolves to the matching named
/// `ConcreteType::Struct` / `ConcreteType::Enum`. Composite annotations
/// (`Array<T>`, `Option<T>`, `Result<T, E>`, `HashMap<K, V>`, tuples)
/// recurse. Returns `None` for anything not fully resolvable — the caller
/// then simply records nothing (best-effort, never fabricates).
pub fn declared_annotation_concrete_type(
    compiler: &BytecodeCompiler,
    ann: &TypeAnnotation,
) -> Option<ConcreteType> {
    use shape_value::v2::concrete_type::{EnumLayoutId, StructLayoutId};
    match ann {
        TypeAnnotation::Basic(name) => concrete_type_from_name(name).or_else(|| {
            named_user_type_concrete(compiler, name, StructLayoutId(0), EnumLayoutId(0))
        }),
        TypeAnnotation::Reference(path) if !path.is_qualified() => {
            let n = path.as_str();
            concrete_type_from_name(n).or_else(|| {
                named_user_type_concrete(compiler, n, StructLayoutId(0), EnumLayoutId(0))
            })
        }
        TypeAnnotation::Generic { name, args } => {
            let base = name.as_str();
            match base {
                "Array" | "Vec" if args.len() == 1 => Some(ConcreteType::Array(Box::new(
                    declared_annotation_concrete_type(compiler, &args[0])?,
                ))),
                "HashMap" | "Map" if args.len() == 2 => Some(ConcreteType::HashMap(
                    Box::new(declared_annotation_concrete_type(compiler, &args[0])?),
                    Box::new(declared_annotation_concrete_type(compiler, &args[1])?),
                )),
                "Option" if args.len() == 1 => Some(ConcreteType::Option(Box::new(
                    declared_annotation_concrete_type(compiler, &args[0])?,
                ))),
                "Result" if args.len() == 2 => Some(ConcreteType::Result(
                    Box::new(declared_annotation_concrete_type(compiler, &args[0])?),
                    Box::new(declared_annotation_concrete_type(compiler, &args[1])?),
                )),
                // T1 sub-case (b) (strict-flip, 2026-06-20): `Result<T>` shorthand
                // (implicit AnyError) — see the matching arm in
                // `concrete_type_from_annotation`. Void err placeholder is sound
                // (`?` discards the err type; the `!!` arm maps it).
                "Result" if args.len() == 1 => Some(ConcreteType::Result(
                    Box::new(declared_annotation_concrete_type(compiler, &args[0])?),
                    Box::new(ConcreteType::Void),
                )),
                _ => None,
            }
        }
        TypeAnnotation::Array(inner) => Some(ConcreteType::Array(Box::new(
            declared_annotation_concrete_type(compiler, inner)?,
        ))),
        TypeAnnotation::Tuple(items) => {
            let elems: Option<Vec<ConcreteType>> = items
                .iter()
                .map(|t| declared_annotation_concrete_type(compiler, t))
                .collect();
            Some(ConcreteType::Tuple(elems?))
        }
        TypeAnnotation::Void => Some(ConcreteType::Void),
        _ => None,
    }
}

/// v0.3 WS-6 — resolve a bare type name to a named struct / enum
/// `ConcreteType` when it is a registered user type. Returns `None` for an
/// unknown name.
fn named_user_type_concrete(
    compiler: &BytecodeCompiler,
    name: &str,
    struct_layout: shape_value::v2::concrete_type::StructLayoutId,
    enum_layout: shape_value::v2::concrete_type::EnumLayoutId,
) -> Option<ConcreteType> {
    if compiler.struct_types.contains_key(name) {
        return Some(ConcreteType::named_struct(name, struct_layout));
    }
    if compiler
        .type_tracker
        .schema_registry()
        .get(name)
        .map(|schema| schema.get_enum_info().is_some())
        .unwrap_or(false)
    {
        return Some(ConcreteType::named_enum(name, enum_layout));
    }
    None
}

/// Map a Shape type-annotation identifier to its `ConcreteType`. Recognises
/// the builtin primitive scalar names; returns `None` for unknown identifiers.
fn concrete_type_from_name(name: &str) -> Option<ConcreteType> {
    match name {
        "int" | "i64" => Some(ConcreteType::I64),
        "i32" => Some(ConcreteType::I32),
        "i16" => Some(ConcreteType::I16),
        "i8" => Some(ConcreteType::I8),
        "u64" => Some(ConcreteType::U64),
        "u32" => Some(ConcreteType::U32),
        "u16" => Some(ConcreteType::U16),
        "u8" => Some(ConcreteType::U8),
        "number" | "f64" => Some(ConcreteType::F64),
        "bool" => Some(ConcreteType::Bool),
        "string" => Some(ConcreteType::String),
        "decimal" => Some(ConcreteType::Decimal),
        "void" | "Void" => Some(ConcreteType::Void),
        _ => None,
    }
}

/// Whether `annotation` (or any of its sub-annotations) names one of the
/// generic parameters in `generics`.
fn annotation_mentions_any(annotation: &TypeAnnotation, generics: &[&str]) -> bool {
    match annotation {
        TypeAnnotation::Basic(name) => generics.iter().any(|g| *g == name.as_str()),
        TypeAnnotation::Reference(path) => generics.iter().any(|g| *g == path.as_str()),
        TypeAnnotation::Borrow { inner, .. } => annotation_mentions_any(inner, generics),
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
            ConcreteType::Tuple(actual_items) if actual_items.len() == items.len() => items
                .iter()
                .zip(actual_items.iter())
                .all(|(ann, ct)| unify_annotation_with_concrete(ann, ct, generics, bindings)),
            _ => !items.iter().any(|t| annotation_mentions_any(t, generics)),
        },
        TypeAnnotation::Function {
            params: _,
            returns: _,
        } => {
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
        // A borrow `&T` / `&mut T` (R1) unwraps transparently to its
        // referent for monomorphization — references carry no distinct
        // `ConcreteType` carrier, so unify the inner annotation against the
        // actual value (a `&T` param bound by a `T`-typed arg binds `T`).
        TypeAnnotation::Borrow { inner, .. } => {
            unify_annotation_with_concrete(inner, actual, generics, bindings)
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
/// call. Uses explicit binding facts, runtime inference facts, tracker metadata,
/// and literal-shape inference.
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

/// Resolve a captured binding's `ConcreteType` from the canonical runtime
/// inference facts. The VM stores only the binding slot's AST span; the type
/// itself is owned by `InferenceFacts`.
pub fn binding_fact_capture_type(compiler: &BytecodeCompiler, name: &str) -> Option<ConcreteType> {
    if let Some(local_idx) = compiler_resolve_local(compiler, name) {
        return compiler
            .local_binding_spans
            .get(&local_idx)
            .copied()
            .and_then(|span| compiler.inference_facts.binding_type(span))
            .and_then(|ty| concrete_type_from_inference_fact(compiler, ty));
    }

    if let Some(binding_idx) = compiler_resolve_module_binding(compiler, name) {
        return compiler
            .module_binding_spans
            .get(&binding_idx)
            .copied()
            .and_then(|span| compiler.inference_facts.binding_type(span))
            .and_then(|ty| concrete_type_from_inference_fact(compiler, ty));
    }

    None
}

fn concrete_type_from_inference_fact(
    compiler: &BytecodeCompiler,
    ty: &Type,
) -> Option<ConcreteType> {
    let canonical = ty.canonicalize();
    match &canonical {
        Type::Concrete(ann) => declared_annotation_concrete_type(compiler, ann)
            .or_else(|| concrete_type_from_annotation(ann, &HashMap::new())),
        Type::Generic { base, args } => {
            let base_name = inference_fact_base_name(base)?;
            match base_name {
                "Array" | "Vec" if args.len() == 1 => Some(ConcreteType::Array(Box::new(
                    concrete_type_from_inference_fact(compiler, &args[0])?,
                ))),
                "HashMap" | "Map" if args.len() == 2 => Some(ConcreteType::HashMap(
                    Box::new(concrete_type_from_inference_fact(compiler, &args[0])?),
                    Box::new(concrete_type_from_inference_fact(compiler, &args[1])?),
                )),
                "HashSet" | "Set" if args.len() == 1 => Some(ConcreteType::HashSet(Box::new(
                    concrete_type_from_inference_fact(compiler, &args[0])?,
                ))),
                "Deque" if args.len() == 1 => Some(ConcreteType::Deque(Box::new(
                    concrete_type_from_inference_fact(compiler, &args[0])?,
                ))),
                "PriorityQueue" if args.len() <= 1 => Some(ConcreteType::PriorityQueue),
                "Option" if args.len() == 1 => Some(ConcreteType::Option(Box::new(
                    concrete_type_from_inference_fact(compiler, &args[0])?,
                ))),
                "Result" if args.len() == 2 => {
                    let ok = concrete_type_from_inference_fact(compiler, &args[0])?;
                    let err = if inference_fact_is_any_error(&args[1]) {
                        ConcreteType::Void
                    } else {
                        concrete_type_from_inference_fact(compiler, &args[1])?
                    };
                    Some(ConcreteType::Result(Box::new(ok), Box::new(err)))
                }
                "Result" if args.len() == 1 => Some(ConcreteType::Result(
                    Box::new(concrete_type_from_inference_fact(compiler, &args[0])?),
                    Box::new(ConcreteType::Void),
                )),
                _ => None,
            }
        }
        Type::Function { .. } => Some(ConcreteType::Function(
            shape_value::v2::concrete_type::FunctionTypeId(0),
        )),
        Type::Variable(_) | Type::Constrained { .. } => None,
    }
}

fn inference_fact_is_any_error(ty: &Type) -> bool {
    match ty.canonicalize() {
        Type::Concrete(TypeAnnotation::Reference(path)) => path.as_str() == "AnyError",
        Type::Concrete(TypeAnnotation::Basic(name)) => name == "AnyError",
        _ => false,
    }
}

fn inference_fact_base_name(ty: &Type) -> Option<&str> {
    match ty {
        Type::Concrete(TypeAnnotation::Reference(path)) => Some(path.as_str()),
        Type::Concrete(TypeAnnotation::Basic(name)) => Some(name.as_str()),
        _ => None,
    }
}

fn compiler_resolve_module_binding(compiler: &BytecodeCompiler, name: &str) -> Option<u16> {
    if let Some(&idx) = compiler.module_bindings.get(name) {
        return Some(idx);
    }
    for module_path in compiler.module_scope_stack.iter().rev() {
        let candidate = format!("{}::{}", module_path, name);
        if let Some(&idx) = compiler.module_bindings.get(&candidate) {
            return Some(idx);
        }
    }
    None
}

pub fn concrete_type_for_expr(compiler: &BytecodeCompiler, expr: &Expr) -> Option<ConcreteType> {
    match expr {
        Expr::Literal(literal, _) => literal_concrete_type(literal),

        Expr::Identifier(name, span) => compiler
            .resolved_expr_types
            .get(span)
            .and_then(|ty| concrete_type_from_inference_fact(compiler, ty))
            .or_else(|| identifier_concrete_type(compiler, name)),

        Expr::Array(elements, _) => {
            // U4-6a: the array element ConcreteType is derived STRUCTURALLY from
            // the literal's own elements — the first element's `ConcreteType`,
            // provided every element agrees (a homogeneous literal). A
            // heterogeneous or unresolvable literal yields `None` (the resolver
            // then falls back to the generic template — no fabrication). The
            // former per-span `array_element_types` side-table (a frozen
            // projection populated only AFTER `compile_expr_array` ran) is
            // deleted: this structural recursion is the single source of truth
            // and is reached at generic call sites BEFORE arg compilation, where
            // the side-table was always empty anyway.
            let mut elem_ct: Option<ConcreteType> = None;
            for el in elements {
                match concrete_type_for_expr(compiler, el) {
                    Some(ct) => match &elem_ct {
                        Some(existing) if existing != &ct => return None,
                        Some(_) => {}
                        None => elem_ct = Some(ct),
                    },
                    None => return None,
                }
            }
            elem_ct.map(|elem| ConcreteType::Array(Box::new(elem)))
        }

        Expr::UnaryOp { operand, .. } => {
            // Unary ops preserve the operand's type (Neg / Not / BitNot).
            concrete_type_for_expr(compiler, operand)
        }

        // Nested-index element recovery (v0.3.3 B4, references slice D2):
        // a single `arr[i]` or a nested `m[r][c]` recovers its element
        // ConcreteType by descending into the object's array ConcreteType
        // and unwrapping ONE `Array` layer per index op. For `m[r][c]` the
        // recursion resolves the object `m[r]` to `Array<int>` (one unwrap
        // off `Array<Array<int>>`), then this arm unwraps the second layer to
        // `int`. The object's ConcreteType IS the proof (ADR-006 §2.7.5):
        // an `Array<Array<int>>`-annotated `m` records
        // `ConcreteType::Array(Array(I64))` via `identifier_concrete_type`;
        // an object that does not resolve to an array ConcreteType yields
        // `None` here, so the operand stays unproven and the binop emitter
        // raises a clean compile error (no fabrication, no `Any` fallback).
        // Slice access (`end_index: Some(_)`) keeps the array shape, not the
        // element, so it is excluded.
        Expr::IndexAccess {
            object,
            end_index: None,
            ..
        } => match concrete_type_for_expr(compiler, object) {
            Some(ConcreteType::Array(elem)) => Some(*elem),
            _ => None,
        },

        // Construction strict-typing close (2026-06-05): a binary-op element
        // (`[x, x * 10]`, `[a + b, c - d]`) has a statically-known result
        // ConcreteType when both operands resolve to the SAME concrete type
        // (no coercion per CLAUDE.md §Type-System-Rules). Arithmetic / bitwise
        // ops preserve the operand type; comparison / logical / fuzzy ops
        // yield `bool` regardless of operand type. Without this arm an array
        // literal whose elements are arithmetic expressions
        // (`fn pair(x: int) -> Array<int> { [x, x * 10] }`) could not resolve
        // its element `TypedArrayKind` and surfaced "cannot infer element
        // type" — the inner-literal element-type-inference hole behind the
        // flatMap consumer surface. Per ADR-006 §2.7.5 stamp-at-compile-time:
        // the result type is the operands' proven type, not a runtime probe.
        Expr::BinaryOp {
            left, op, right, ..
        } => {
            use shape_ast::ast::BinaryOp;
            match op {
                // Comparison / logical / fuzzy → bool (operands need not
                // resolve here; the result kind is bool unconditionally).
                BinaryOp::Greater
                | BinaryOp::Less
                | BinaryOp::GreaterEq
                | BinaryOp::LessEq
                | BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::FuzzyEqual
                | BinaryOp::FuzzyGreater
                | BinaryOp::FuzzyLess
                | BinaryOp::And
                | BinaryOp::Or => Some(ConcreteType::Bool),
                // Arithmetic / bitwise → operand type (both must agree; a
                // mismatch or unresolved operand yields None, falling back to
                // the generic path / clean compile error — no fabrication).
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::Pow
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitShl
                | BinaryOp::BitShr => {
                    let lt = concrete_type_for_expr(compiler, left)?;
                    let rt = concrete_type_for_expr(compiler, right)?;
                    if lt == rt { Some(lt) } else { None }
                }
                // NullCoalesce (`a ?? b`): result is the non-null branch type.
                // Both branches should agree; resolve the right (default) type.
                BinaryOp::NullCoalesce => {
                    let rt = concrete_type_for_expr(compiler, right)?;
                    Some(rt)
                }
                // ErrorContext (`expr !! ctx`) always yields a `Result<T, E>`:
                //   - `Result<T, E> !! ctx` → `Result<T, E>` (success + error
                //     preserved)
                //   - `Option<T> !! ctx` / `T !! ctx` → `Result<T, Void>`
                // Resolving the success ConcreteType here (rather than falling
                // back to None) lets a `let v = (g() !! ctx)?` binding record
                // `v`'s success type, so a downstream `v + 1` sees the operand
                // type instead of `unknown` (finding 5). The success type is
                // the proof (ADR-006 §2.7.5) — an unresolved left operand
                // yields None and a clean compile error downstream, no
                // fabrication.
                BinaryOp::ErrorContext => {
                    let lt = concrete_type_for_expr(compiler, left)?;
                    match lt {
                        ConcreteType::Result(ok, err) => Some(ConcreteType::Result(ok, err)),
                        ConcreteType::Option(inner) => {
                            Some(ConcreteType::Result(inner, Box::new(ConcreteType::Void)))
                        }
                        other => Some(ConcreteType::Result(
                            Box::new(other),
                            Box::new(ConcreteType::Void),
                        )),
                    }
                }
                // Pipe is opaque here (callee-dependent); fall back to None.
                BinaryOp::Pipe => None,
            }
        }

        // Phase 4b Round 3 Surface-1B LANG-W13-3-double-filter-chain:
        // Chained method-call receivers like `v.filter(|x|...).filter(|y|...)`
        // need the inner `.filter(...)`'s return ConcreteType to resolve at
        // the outer call site for monomorphization to succeed. The
        // `specialized_call_return_concrete_type` helper above already
        // chains through `monomorphized_method_call_sites[(span,
        // current_function)] → specialized_idx → function_defs[
        // specialized_name].return_type` for exactly this purpose — but
        // until now it was wired only at the `let intermediate = recv.map(
        // ...)` let-binding site (cluster-2-cw-IC-class-c), missing the
        // inline-chain receiver shape.
        //
        // Without this arm `try_monomorphize_method_call` at the outer
        // `.filter` call site receives `receiver_ct = None` from
        // `concrete_type_for_expr(receiver)` (since `receiver` is an
        // `Expr::MethodCall`, not an Identifier or Array literal), then
        // bails to the generic-template fall-through path. The VM-side
        // generic dispatch enters an infinite loop on the chained
        // call shape (single-timeout / hang at the c08 seed); the JIT
        // emits an untyped `print` Call-terminator and reads raw bits
        // as garbage.
        //
        // Per ADR-006 §2.7.5 stamp-at-compile-time: the inner call's
        // specialized callee's substituted return-type annotation IS the
        // proof — same chain as the let-binding site at statements.rs:4931.
        // No tag-bit decode, no runtime probe, no fabricated default.
        Expr::MethodCall {
            receiver, method, ..
        } => {
            // The `DateTime` namespace constructors all yield a `DateTime`
            // value (datetime book chapter). Recording `ConcreteType::DateTime`
            // for `let a = DateTime.parse(..)` is what lets the binding's
            // tracker type-name surface "DateTime" at a downstream operator
            // site (`a - b`, `a + 3d`) so the temporal Add/Sub dispatch fires;
            // without it the operand stays `unknown` and strict typing rejects.
            // Bounded to a bare-identifier `DateTime` receiver that is NOT a
            // user struct / alias / local (mirrors the inference-engine
            // `is_namespace_constructor` guard). The name IS the proof
            // (ADR-006 §2.7.5) — no runtime probe.
            if let Expr::Identifier(recv_name, _) = receiver.as_ref() {
                if recv_name == "DateTime"
                    && matches!(
                        method.as_str(),
                        "now" | "utc" | "parse" | "from_epoch" | "from_parts" | "from_unix_secs"
                    )
                    && compiler.resolve_local(recv_name).is_none()
                    && !compiler.module_bindings.contains_key(recv_name.as_str())
                {
                    return Some(ConcreteType::DateTime);
                }
            }
            // DateTime INSTANCE methods on a receiver already proven to be a
            // `DateTime` (datetime book chapter "Method Reference"). The
            // namespace-constructor arm above proves `let dt =
            // DateTime.parse(..)` is a `DateTime`; this arm lets a downstream
            // `dt.year()` / `dt.format(..)` / `dt.add_days(1)` surface its
            // documented return `ConcreteType` so element-type-sensitive
            // contexts (a `[dt.format("%Y-%m-%d")]` array-literal element, an
            // `Array<string>` accumulation `acc + [..]`) can prove a homogeneous
            // element kind instead of falling to "cannot infer element type".
            //
            // The receiver's proven `DateTime` ConcreteType IS the proof
            // (ADR-006 §2.7.5) — recovered structurally via the recursive
            // `concrete_type_for_expr`, never a runtime probe. The
            // (method -> return) table mirrors the strict checker's
            // `register_datetime_methods` (`method_table.rs`) verbatim; an
            // unlisted method yields `None` and falls through unchanged (no
            // fabrication, no Bool-default — a method whose return kind is not
            // documented here is simply opaque to this resolver).
            if matches!(
                concrete_type_for_expr(compiler, receiver),
                Some(ConcreteType::DateTime)
            ) {
                match method.as_str() {
                    // Component / day-info / timestamp accessors — `-> int`.
                    "year" | "month" | "day" | "hour" | "minute" | "second" | "millisecond"
                    | "microsecond" | "day_of_week" | "day_of_year" | "week_of_year"
                    | "unix_timestamp" | "to_unix_millis" => {
                        return Some(ConcreteType::I64);
                    }
                    // Day-info / comparison predicates — `-> bool`.
                    "is_weekday" | "is_weekend" | "is_before" | "is_after" | "is_same_day" => {
                        return Some(ConcreteType::Bool);
                    }
                    // Formatting / timezone-name / offset — `-> string`.
                    "format" | "iso8601" | "rfc2822" | "timezone" | "offset" => {
                        return Some(ConcreteType::String);
                    }
                    // Timezone conversions + arithmetic — `-> DateTime`.
                    "to_utc" | "to_local" | "to_timezone" | "add_days" | "add_hours"
                    | "add_minutes" | "add_seconds" | "add_months" | "add" | "sub" => {
                        return Some(ConcreteType::DateTime);
                    }
                    // Any other method is opaque to this resolver — fall through.
                    _ => {}
                }
            }
            // First: a monomorphized stdlib method call records its substituted
            // return annotation at the call site (the `.map`/`.filter` chain
            // path). When present, that IS the proof — use it verbatim.
            if let Some(ct) = specialized_call_return_concrete_type(compiler, expr) {
                return Some(ct);
            }
            // R3-elemerasure (strict-flip): the builtin (PHF) array methods that
            // return `Self` (`sort`/`reverse`/`take`/`drop`/`slice`/`skip`/
            // `clone`/`unique`/`flatten`/`distinct`/`sortBy`/`concat`) or the
            // receiver element type (`first`/`last`/`pop`/`find`) are NOT
            // monomorphized stdlib functions, so the chain above finds nothing
            // and the result type was lost — a downstream `.map(|x| x*x)` saw
            // `x: unknown` and surfaced "Cannot infer types for binary
            // operation". Recover the result `ConcreteType` from the receiver's
            // proven `ConcreteType`, driven by the method's REGISTERED
            // `GenericMethodSignature` return shape (no hardcoded method list —
            // the signature IS the proof, per ADR-006 §2.7.5). Only fires for an
            // Array receiver whose element type is already known; an unproven
            // receiver yields `None` (no fabrication).
            method_call_receiver_derived_concrete_type(compiler, receiver, method)
        }

        // v0.3 ε-4 generic-fn-chain: a free-function-call argument like the
        // inner `id(42)` in `id(id(id(42)))`. Without this arm
        // `concrete_type_for_expr` returns `None` for the chained argument, so
        // `resolve_call_site_type_args` cannot bind the outer call's type
        // params and `try_monomorphize_free_function_call` falls back to the
        // *unspecialized* generic template. Generic templates have empty
        // (body-skipped) bytecode — dispatching onto one hangs the VM.
        //
        // `function_call_return_concrete_type` resolves the callee's return
        // annotation, substituting type-param bindings for a generic callee.
        // The recursion is structural over the finite argument AST (each step
        // descends into strictly-smaller sub-expressions), so it always
        // terminates — including for self-recursive callees, since it walks
        // only the call's *argument* expressions, never the callee body.
        Expr::FunctionCall { name, args, .. } => {
            // v0.3 WS-6 generic-arg-fix: the trinity constructors `Some` /
            // `Ok` / `Err` parse as `Expr::FunctionCall` (they map to
            // `BuiltinFunction::SomeCtor` / `OkCtor` / `ErrCtor`, not to a
            // user `function_defs` entry). Resolve them to the matching
            // `Option<T>` / `Result<T, E>` ConcreteType from the payload
            // expression so a generic free-function call like
            // `id(Some(5))` can bind its type param. A genuinely
            // type-ambiguous payload (`Some(<unresolvable>)`) returns
            // `None`, preserving the WS-2 clean-compile-error contract.
            match name.as_str() {
                "Some" if args.len() == 1 => concrete_type_for_expr(compiler, &args[0])
                    .map(|inner| ConcreteType::Option(Box::new(inner))),
                "Ok" if args.len() == 1 => {
                    concrete_type_for_expr(compiler, &args[0]).map(|inner| {
                        ConcreteType::Result(Box::new(inner), Box::new(ConcreteType::Void))
                    })
                }
                "Err" if args.len() == 1 => {
                    concrete_type_for_expr(compiler, &args[0]).map(|inner| {
                        ConcreteType::Result(Box::new(ConcreteType::Void), Box::new(inner))
                    })
                }
                // `None` carries no payload — its `Option<T>` element type
                // is genuinely unresolvable without call-site context.
                // Returning `None` keeps `id(None)` a clean compile error.
                "None" => None,
                _ => function_call_return_concrete_type(compiler, name, args),
            }
        }

        // v0.3 WS-6 generic-arg-fix: a struct literal `P { a: 7 }` is a
        // fully type-known argument expression. Resolve it to the named
        // struct ConcreteType so a generic free-function call like
        // `id(P { a: 7 })` can bind its type param. `ConcreteType::Struct`
        // carries a `StructLayoutId` placeholder; the load-bearing type
        // identity for monomorphization is the struct *name*, threaded via
        // `struct_concrete_type` into the resolver's name-aware path.
        Expr::StructLiteral { type_name, .. } => {
            struct_or_enum_concrete_type(compiler, type_name.name())
        }

        // v0.3 WS-6 generic-arg-fix: an enum constructor `Color::Red` /
        // `Shape::Circle(2.0)` is a fully type-known argument. Resolve it to
        // the named enum ConcreteType. The user-defined-enum case maps to
        // `ConcreteType::Enum`; the trinity enum names `Option` / `Result`
        // are handled by the `Some` / `Ok` / `Err` FunctionCall arms above
        // and the dedicated trinity arm here.
        Expr::EnumConstructor {
            enum_name,
            variant,
            payload,
            ..
        } => enum_constructor_concrete_type(compiler, enum_name.name(), variant, payload),

        // `@"..."` DateTime literal (`Expr::DateTime`) / time reference
        // (`Expr::TimeRef`). A datetime literal IS a `DateTime` value (datetime
        // book chapter §DateTime Literals) — the literal form is the proof
        // (ADR-006 §2.7.5), no runtime probe. This lets `let dt = @"..."` record
        // `ConcreteType::DateTime` in the tracker so a downstream `dt.year()` /
        // `dt.format(..)` receiver resolves to DateTime (parity with the
        // `DateTime.parse(..)` constructor binding) and its method result type
        // surfaces for element-type-sensitive contexts.
        Expr::DateTime(_, _) | Expr::TimeRef(_, _) => Some(ConcreteType::DateTime),

        // `expr?` propagates the error/none and yields the success type. The
        // inner expression's ConcreteType is `Result<T, E>` or `Option<T>`;
        // unwrap to `T` so a `let v = (g() !! ctx)?` binding records `v: T`
        // and a downstream `v + 1` resolves the operand (finding 5). An inner
        // type that is neither Result nor Option (or unresolved) yields None —
        // the `?` would be a compile error there anyway, no fabrication.
        Expr::TryOperator(inner, _) => match concrete_type_for_expr(compiler, inner)? {
            ConcreteType::Result(ok, _) => Some(*ok),
            ConcreteType::Option(inner_ct) => Some(*inner_ct),
            _ => None,
        },

        // T1 sub-case (a) (strict-flip, 2026-06-20): a field read whose object
        // resolves to a NAMED struct `ConcreteType::Struct` (the load-bearing
        // shape being `rs[0].len` where `rs: Vec<Run>` — the `IndexAccess` arm
        // above already unwraps `Array<Run>` -> `Struct(Run)`). Without this arm
        // the field-result `ConcreteType` was lost, so `rs[0].len + 1` saw the
        // operand as `unknown` and the binop emitter rejected it. Resolve the
        // struct's schema field type and map it to a `ConcreteType`. The struct
        // name + schema field type ARE the proof (ADR-006 §2.7.5); an unnamed /
        // unregistered struct, or a field with no statically-mappable kind,
        // yields `None` (clean fall-through, no fabrication).
        Expr::PropertyAccess {
            object, property, ..
        } => match concrete_type_for_expr(compiler, object)? {
            ConcreteType::Struct(layout) => {
                let struct_name = layout.name.as_ref()?;
                let schema = compiler.type_tracker.schema_registry().get(struct_name)?;
                let field = schema.get_field(property)?;
                field_type_to_concrete(&field.field_type)
            }
            _ => None,
        },

        // Anything else (member accesses, closures, …) is opaque until we
        // have richer side-tables. Returning None lets the resolver fall back
        // to the generic template.
        _ => None,
    }
}

/// v0.3 WS-6 — best-effort `ConcreteType` for a named user type referenced
/// by a struct literal or unit-payload enum constructor.
///
/// The bytecode compiler records every user `type` / `enum` declaration in
/// `struct_types` (structs) and the schema registry (enums). When `name`
/// resolves to a known user type the helper produces the matching
/// `ConcreteType::Struct` / `ConcreteType::Enum`. The `StructLayoutId` /
/// `EnumLayoutId` are placeholders — the monomorphization key and the
/// substituted annotation are name-carried (see `concrete_to_annotation`'s
/// WS-6 named-type arm), so the placeholder id never participates in type
/// identity.
///
/// Returns `None` for an unknown name; the resolver then falls back to the
/// generic template (no fabrication).
fn struct_or_enum_concrete_type(compiler: &BytecodeCompiler, name: &str) -> Option<ConcreteType> {
    use shape_value::v2::concrete_type::{EnumLayoutId, StructLayoutId};
    named_user_type_concrete(compiler, name, StructLayoutId(0), EnumLayoutId(0))
}

/// v0.3 WS-6 — best-effort `ConcreteType` for an `Expr::EnumConstructor`.
///
/// The trinity enums `Option` / `Result` carry parametric inner types; their
/// constructor payloads are resolved to the matching `ConcreteType::Option`
/// / `ConcreteType::Result`. A user-defined enum resolves to the named
/// `ConcreteType::Enum`. A trinity constructor whose payload type cannot be
/// resolved (e.g. `Option::Some` of an opaque expression) returns `None`,
/// preserving the clean-compile-error contract for genuinely ambiguous
/// arguments.
fn enum_constructor_concrete_type(
    compiler: &BytecodeCompiler,
    enum_name: &str,
    variant: &str,
    payload: &shape_ast::ast::EnumConstructorPayload,
) -> Option<ConcreteType> {
    use shape_ast::ast::EnumConstructorPayload;
    match enum_name {
        "Option" => match (variant, payload) {
            ("Some", EnumConstructorPayload::Tuple(items)) if items.len() == 1 => {
                concrete_type_for_expr(compiler, &items[0])
                    .map(|inner| ConcreteType::Option(Box::new(inner)))
            }
            // `Option::None` carries no payload — genuinely ambiguous.
            ("None", _) => None,
            _ => None,
        },
        "Result" => match (variant, payload) {
            ("Ok", EnumConstructorPayload::Tuple(items)) if items.len() == 1 => {
                concrete_type_for_expr(compiler, &items[0]).map(|inner| {
                    ConcreteType::Result(Box::new(inner), Box::new(ConcreteType::Void))
                })
            }
            ("Err", EnumConstructorPayload::Tuple(items)) if items.len() == 1 => {
                concrete_type_for_expr(compiler, &items[0]).map(|inner| {
                    ConcreteType::Result(Box::new(ConcreteType::Void), Box::new(inner))
                })
            }
            _ => None,
        },
        _ => struct_or_enum_concrete_type(compiler, enum_name),
    }
}

/// v0.3 ε-4 — best-effort return [`ConcreteType`] for a free-function call
/// `name(args...)`, used so a function-call *argument* (e.g. the inner
/// `id(42)` in a `id(id(42))` chain) can resolve a concrete type at the outer
/// monomorphization site.
///
/// For a non-generic callee the declared return annotation is reduced
/// directly. For a generic callee the type-param bindings are recovered from
/// the call-site argument types (the same unification
/// [`resolve_call_site_type_args`] performs) and substituted into the return
/// annotation before reduction — so `id<T>(x: T) -> T` called with an
/// `int`-typed argument resolves to `ConcreteType::I64`.
///
/// Returns `None` whenever a link is missing (unknown callee, foreign/`Result`
/// shorthand return shape, an unresolved generic, …); the caller then falls
/// back to the generic-template path. Never fabricates a type.
///
/// Termination: the only recursion is the `concrete_type_for_expr` call on
/// each argument expression, which descends strictly into the finite argument
/// AST. A chain `id(id(id(42)))` recurses argument-by-argument to the literal
/// `42` and unwinds; a self-recursive callee `fn f<T>(x:T)->T { f(x) }`
/// resolves only `f`'s argument list, never re-entering `f`'s body.
fn function_call_return_concrete_type(
    compiler: &BytecodeCompiler,
    name: &str,
    args: &[Expr],
) -> Option<ConcreteType> {
    let func_def = compiler.function_defs.get(name)?;
    let Some(return_annotation) = func_def.return_type.as_ref() else {
        // strict-flip S1 (call-site HOF return propagation, 2026-06-22): the
        // callee has NO declared return annotation. Its return type is not
        // genuinely `unknown` whenever the body's tail is a call of a
        // function-valued parameter (`fn apply(f, x) { f(x) }`) and the actual
        // argument passed for that parameter is a concrete callable — then the
        // result is that callable's PROVEN return type (`apply(ret_num, 3.0)`
        // ⇒ `number`, because `ret_num: number -> number`). Resolve it here so
        // a downstream `let bad: int = apply(ret_num, 3.0)` fails NATURALLY
        // (number != int) and the matched `let r: number = …` binds cleanly.
        // Any non-HOF / unresolvable shape yields `None` — the binding then
        // hits the let-annotation Unknown-accept guard (FIX B) instead of
        // silently laundering `unknown` into a concrete slot.
        return hof_unannotated_call_return_concrete_type(compiler, func_def, args);
    };

    // Collect the declared (non-const) type-param names.
    let type_params: Vec<String> = func_def
        .type_params
        .as_ref()
        .map(|tps| {
            tps.iter()
                .filter(|tp| !tp.is_const())
                .map(|tp| tp.name().to_string())
                .collect()
        })
        .unwrap_or_default();

    if type_params.is_empty() {
        // Non-generic callee — reduce the return annotation directly.
        return concrete_type_from_annotation(return_annotation, &HashMap::new());
    }

    // Generic callee — recover type-param bindings from the argument types,
    // then substitute them into the return annotation.
    let arg_types = extract_arg_concrete_types(compiler, args);
    let resolution = resolve_call_site_type_args(compiler, name, &arg_types, &type_params)?;
    if resolution.type_args.len() != type_params.len() {
        return None;
    }
    let bindings: HashMap<String, ConcreteType> = type_params
        .iter()
        .cloned()
        .zip(resolution.type_args.iter().cloned())
        .collect();
    concrete_type_from_annotation(return_annotation, &bindings)
}

/// strict-flip S1 (call-site HOF return propagation, 2026-06-22): resolve the
/// PROVEN result `ConcreteType` of a call to a function that has NO declared
/// return annotation but whose body's tail is a call of a function-valued
/// parameter.
///
/// The load-bearing shape is `fn apply(f, x) { f(x) }` invoked as
/// `apply(ret_num, 3.0)`. `apply`'s static return is `unknown` (no annotation),
/// but the body returns `f(x)`, `f` is bound to the concrete callable `ret_num`
/// (a `number -> number` fn), and `x` is bound to `3.0` (number) — so `f(x)` is
/// `ret_num(number)` = `number`. The result is therefore PROVEN `number`, not
/// `unknown`. Recovering it lets `let bad: int = apply(ret_num, 3.0)` fail
/// naturally (`number != int`) while `let r: number = …` binds cleanly.
///
/// Per ADR-006 §2.7.5 stamp-at-compile-time: the proof is the actual callable
/// argument's return type, resolved structurally — no runtime probe, no
/// coercion, no fabrication. Any link that is not statically provable yields
/// `None`; the binding then meets the let-annotation Unknown-accept guard
/// (FIX B) rather than laundering `unknown` into a concrete slot. NO silent
/// widen, NO `int`/`number` unify.
fn hof_unannotated_call_return_concrete_type<'c>(
    compiler: &'c BytecodeCompiler,
    callee: &'c shape_ast::ast::FunctionDef,
    args: &'c [Expr],
) -> Option<ConcreteType> {
    // Generic callees route through the type-param substitution path; this
    // helper only handles the un-annotated, value-param HOF.
    if callee
        .type_params
        .as_ref()
        .is_some_and(|tps| !tps.is_empty())
    {
        return None;
    }
    if callee.params.len() != args.len() {
        return None;
    }

    // Build the resolution environment: each callee param NAME maps to either
    // the concrete TYPE of its scalar call-site argument, or — when the argument
    // names a concrete non-generic user function — that callable's definition.
    // A destructuring/rest param is not a simple forwarding param; the whole
    // resolution bails (conservative `None`).
    let mut env: HashMap<&str, ParamBinding<'c>> = HashMap::new();
    for (p, a) in callee.params.iter().zip(args.iter()) {
        let ident = p.pattern.as_identifier()?;
        let binding = match a {
            // A bare identifier argument MAY name a concrete callable
            // (function-valued param: `apply2(id, ret_num, 3.0)` passes `id`
            // and `ret_num` as callables). Prefer the callable binding; fall
            // back to a concrete-type resolution (the identifier may be a
            // value-typed local).
            Expr::Identifier(arg_name, _) => {
                if let Some(callable) = resolvable_callable(compiler, arg_name) {
                    ParamBinding::Callable(callable)
                } else {
                    ParamBinding::Type(concrete_type_for_expr(compiler, a)?)
                }
            }
            _ => ParamBinding::Type(concrete_type_for_expr(compiler, a)?),
        };
        env.insert(ident, binding);
    }
    if env.len() != callee.params.len() {
        return None;
    }

    // Resolve the callee body's tail expression under this environment. The
    // tail may be `f(x)` (1-level forward) or `g(f(x))` (nested HOF) — both
    // resolve structurally through the param-callable bindings, never a runtime
    // probe (ADR-006 §2.7.5). Any unresolvable link yields `None`.
    let tail = body_tail_expr(&callee.body)?;
    hof_body_expr_concrete_type(compiler, &env, tail)
}

/// A callee parameter's resolution binding inside a HOF body: either the
/// concrete TYPE of a scalar argument, or a concrete callable function the
/// argument names (a function-valued parameter).
enum ParamBinding<'c> {
    Type(ConcreteType),
    Callable(&'c shape_ast::ast::FunctionDef),
}

/// `name` resolves to a concrete, non-generic user function (not a local
/// variable / module binding shadowing it). Returns its definition, or `None`.
fn resolvable_callable<'c>(
    compiler: &'c BytecodeCompiler,
    name: &str,
) -> Option<&'c shape_ast::ast::FunctionDef> {
    if compiler.resolve_local(name).is_some() {
        return None;
    }
    let def = compiler.function_defs.get(name)?;
    if def.type_params.as_ref().is_some_and(|tps| !tps.is_empty()) {
        return None;
    }
    Some(def)
}

/// Resolve the `ConcreteType` of a HOF body-tail expression under an
/// environment mapping the enclosing fn's params to scalar types or callable
/// definitions. A `FunctionCall` whose name is a callable-bound param (or a
/// global concrete fn) is resolved by recovering each inner-argument type in
/// the same environment, then resolving that callable's PROVEN return type
/// (recursively — the callable may itself be un-annotated, e.g. `id`). Scalar
/// shapes (literals, param refs, arithmetic) resolve as before. Any
/// unresolvable shape yields `None` — no fabrication, no `int`/`number` unify.
fn hof_body_expr_concrete_type(
    compiler: &BytecodeCompiler,
    env: &HashMap<&str, ParamBinding<'_>>,
    expr: &Expr,
) -> Option<ConcreteType> {
    use shape_ast::ast::BinaryOp;
    match expr {
        Expr::Literal(lit, _) => literal_concrete_type(lit),

        Expr::Identifier(name, _) => match env.get(name.as_str()) {
            Some(ParamBinding::Type(ct)) => Some(ct.clone()),
            // A param bound to a callable used in value position is not a
            // scalar — unresolvable here.
            Some(ParamBinding::Callable(_)) => None,
            None => concrete_type_for_expr(compiler, expr),
        },

        Expr::UnaryOp { operand, .. } => hof_body_expr_concrete_type(compiler, env, operand),
        Expr::Return(Some(inner), _) => hof_body_expr_concrete_type(compiler, env, inner),

        // The load-bearing arm: a call whose callee NAME is either a
        // callable-bound param (`g(...)`, `f(...)` inside the HOF) or a global
        // concrete fn. Resolve each inner-arg type in this environment, then
        // the callable's proven return type with those arg types.
        Expr::FunctionCall {
            name, args: inner, ..
        } => {
            let callable: &shape_ast::ast::FunctionDef = match env.get(name.as_str()) {
                Some(ParamBinding::Callable(def)) => def,
                Some(ParamBinding::Type(_)) => return None, // scalar called as fn
                None => resolvable_callable(compiler, name)?,
            };
            let mut inner_cts: Vec<ConcreteType> = Vec::with_capacity(inner.len());
            for ia in inner {
                inner_cts.push(hof_body_expr_concrete_type(compiler, env, ia)?);
            }
            unannotated_fn_return_concrete_type(compiler, callable, &inner_cts)
        }

        Expr::BinaryOp {
            left, op, right, ..
        } => match op {
            BinaryOp::Greater
            | BinaryOp::Less
            | BinaryOp::GreaterEq
            | BinaryOp::LessEq
            | BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::FuzzyEqual
            | BinaryOp::FuzzyGreater
            | BinaryOp::FuzzyLess
            | BinaryOp::And
            | BinaryOp::Or => Some(ConcreteType::Bool),
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::Pow
            | BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitShl
            | BinaryOp::BitShr => {
                let lt = hof_body_expr_concrete_type(compiler, env, left);
                let rt = hof_body_expr_concrete_type(compiler, env, right);
                match (lt, rt) {
                    (Some(l), Some(r)) if l == r => Some(l),
                    (Some(l), Some(r)) => adopt_int_literal(&l, left, &r, right),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// Resolve the PROVEN return `ConcreteType` of a non-generic function given the
/// concrete types of its arguments. If the function declares a return
/// annotation, that annotation IS the proof (ADR-006 §2.7.5). Otherwise infer
/// the body tail expression's type with each param seeded to its concrete
/// argument type. Any un-inferable link yields `None` — no fabrication, no
/// `int`/`number` unify.
fn unannotated_fn_return_concrete_type(
    compiler: &BytecodeCompiler,
    func_def: &shape_ast::ast::FunctionDef,
    arg_cts: &[ConcreteType],
) -> Option<ConcreteType> {
    if let Some(ann) = func_def.return_type.as_ref() {
        return concrete_type_from_annotation(ann, &HashMap::new());
    }
    if func_def.params.len() != arg_cts.len() {
        return None;
    }
    let mut param_cts: HashMap<&str, ConcreteType> = HashMap::new();
    for (p, ct) in func_def.params.iter().zip(arg_cts.iter()) {
        let ident = p.pattern.as_identifier()?;
        // An explicit param annotation must AGREE with the supplied concrete
        // type (no silent widen). When it disagrees we cannot soundly resolve
        // — yield `None`.
        if let Some(ann) = p.type_annotation.as_ref() {
            let declared = concrete_type_from_annotation(ann, &HashMap::new())?;
            if declared != *ct {
                return None;
            }
        }
        param_cts.insert(ident, ct.clone());
    }
    let tail = body_tail_expr(&func_def.body)?;
    body_expr_concrete_type(compiler, &param_cts, tail)
}

/// The tail (result) expression of a function/closure body: the inner
/// expression of a trailing `return e`, or a trailing expression statement.
fn body_tail_expr(body: &[Statement]) -> Option<&Expr> {
    match body.last()? {
        Statement::Return(Some(e), _) => Some(e),
        Statement::Expression(e, _) => Some(e),
        _ => None,
    }
}

/// `&`-only structural type resolution of a function body's tail expression,
/// with each param seeded to its proven concrete type. Bounded to the scalar
/// arithmetic / param-reference shapes that arise in un-annotated forwarding
/// helpers; any unrecognised shape yields `None` (clean fall-through, no
/// fabrication, no silent `int`/`number` unify).
fn body_expr_concrete_type(
    compiler: &BytecodeCompiler,
    param_cts: &HashMap<&str, ConcreteType>,
    expr: &Expr,
) -> Option<ConcreteType> {
    use shape_ast::ast::BinaryOp;
    match expr {
        Expr::Literal(lit, _) => literal_concrete_type(lit),
        Expr::Identifier(name, _) => param_cts
            .get(name.as_str())
            .cloned()
            .or_else(|| concrete_type_for_expr(compiler, expr)),
        Expr::UnaryOp { operand, .. } => body_expr_concrete_type(compiler, param_cts, operand),
        Expr::Return(Some(inner), _) => body_expr_concrete_type(compiler, param_cts, inner),
        Expr::BinaryOp {
            left, op, right, ..
        } => match op {
            BinaryOp::Greater
            | BinaryOp::Less
            | BinaryOp::GreaterEq
            | BinaryOp::LessEq
            | BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::FuzzyEqual
            | BinaryOp::FuzzyGreater
            | BinaryOp::FuzzyLess
            | BinaryOp::And
            | BinaryOp::Or => Some(ConcreteType::Bool),
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::Pow
            | BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitShl
            | BinaryOp::BitShr => {
                let lt = body_expr_concrete_type(compiler, param_cts, left);
                let rt = body_expr_concrete_type(compiler, param_cts, right);
                // Numeric-conversion §4 literal adoption (THE RULE user
                // 2026-06-01): a BARE int literal sibling of a `number`
                // operand IS the number literal (`x * 2.0` over `x: number`
                // ⇒ number; `x * 2` over `x: number` ⇒ the bare `2` adopts
                // number). A genuine `int`-typed operand keeps `int != number`
                // (mismatch → `None`, no silent unify).
                match (lt, rt) {
                    (Some(l), Some(r)) if l == r => Some(l),
                    (Some(l), Some(r)) => adopt_int_literal(&l, left, &r, right),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// Numeric-conversion §4 literal adoption for a mismatched arithmetic operand
/// pair: when exactly one side is a bare `int` literal and the other side is
/// `number`, the literal adopts `number` (lossless). Otherwise `None` — `int`
/// and `number` never silently unify.
fn adopt_int_literal(
    lt: &ConcreteType,
    left: &Expr,
    rt: &ConcreteType,
    right: &Expr,
) -> Option<ConcreteType> {
    use shape_ast::ast::Literal;
    let is_int_lit = |e: &Expr| {
        matches!(
            e,
            Expr::Literal(Literal::Int(_), _) | Expr::Literal(Literal::UInt(_), _)
        )
    };
    let number = ConcreteType::F64;
    if *lt == number && *rt == ConcreteType::I64 && is_int_lit(right) {
        return Some(number);
    }
    if *rt == number && *lt == ConcreteType::I64 && is_int_lit(left) {
        return Some(number);
    }
    None
}

/// cluster-2-cw-IC-class-c (Phase 3 cluster-2 Round 3, 2026-05-16):
/// look up the specialized return ConcreteType for an `Expr::MethodCall`
/// initializer whose monomorphization succeeded at bytecode-emission time.
///
/// Chains:
///   `Expr::MethodCall.span + current_function`
///   → `BytecodeProgram.monomorphized_method_call_sites[(span, current_fn)]`
///       (populated by `try_monomorphize_method_call` /
///        `try_monomorphize_method_call_with_closures` per ADR-006 §2.7.5
///        V3-S6b conduit)
///   → specialized FunctionId
///   → `BytecodeCompiler.function_defs[specialized_name].return_type`
///       (the substituted AST annotation written by
///        `substitution::substitute_function_def`)
///   → `concrete_type_from_annotation(annotation)`
///       (the same v2 annotation→ConcreteType reduction used by the MIR
///        resolver's `function_return_concrete_types` build)
///
/// Returns `None` when any link is absent — non-method-call RHS, no
/// monomorphization at this call site, unknown specialized function name,
/// or annotation that doesn't reduce to a v2 ConcreteType. No fabrication.
///
/// Per ADR-006 §2.7.5 stamp-at-compile-time: the specialized return type
/// IS the proof — no runtime decode, no Bool-default, no inference.
/// Per §2.7.7 #9: when proof is unavailable the slot stays unstamped;
/// subsequent receiver-kind classification at the second `.map(...)`
/// surfaces with `NotImplemented(SURFACE)` rather than fabricating.
///
/// Class C territory closure: populates whole-binding `ConcreteType::Array(C)`
/// at let-binding-time so the next statement's
/// `concrete_type_for_expr(receiver_identifier)` chain can reach
/// `identifier_concrete_type`, enabling `try_monomorphize_method_call` to
/// specialize the second `.map(...)` on the now-known `Array<C>` receiver type.
#[derive(Debug, Clone, Copy)]
pub(crate) enum BindingInitializerTarget {
    Local(u16),
    ModuleBinding(u16),
}

pub(crate) fn monomorphized_call_site_return_concrete_type(
    compiler: &BytecodeCompiler,
    span: Span,
) -> Option<ConcreteType> {
    let specialized_idx = *compiler
        .program
        .monomorphized_method_call_sites
        .get(&(span, compiler.current_function))?;
    let specialized_name = compiler
        .program
        .functions
        .get(specialized_idx)
        .map(|f| f.name.clone())?;
    let return_annotation = compiler
        .function_defs
        .get(&specialized_name)
        .and_then(|fd| fd.return_type.as_ref())?;
    crate::compiler::v2_map_emission::concrete_type_from_annotation(return_annotation)
}

pub fn specialized_call_return_concrete_type(
    compiler: &BytecodeCompiler,
    expr: &Expr,
) -> Option<ConcreteType> {
    let span = match expr {
        Expr::MethodCall { span, .. } => *span,
        _ => return None,
    };
    monomorphized_call_site_return_concrete_type(compiler, span)
}

/// U4-6 post-monomorphization binding fact: stamp a let-binding target from the
/// monomorphized method-call return recorded at `(call_span, current_function)`.
///
/// This deliberately uses `BytecodeProgram::monomorphized_method_call_sites` as
/// the compiler-side fact carrier. It does not consult runtime `InferenceFacts`
/// and does not fabricate a fallback when the call-site record or return
/// annotation is absent.
pub(crate) fn stamp_binding_initializer_monomorphized_call_return(
    compiler: &mut BytecodeCompiler,
    target: BindingInitializerTarget,
    init_expr: &Expr,
) -> bool {
    let Some(ConcreteType::Array(elem)) =
        specialized_call_return_concrete_type(compiler, init_expr)
    else {
        return false;
    };

    let concrete_type = ConcreteType::Array(elem.clone());
    let tracker_name = tracker_name_for_array_element(elem.as_ref());
    record_binding_concrete_fact(
        compiler,
        target,
        concrete_type.clone(),
        BindingConcreteFactSource::MonomorphizedCallReturn,
    );
    match target {
        BindingInitializerTarget::Local(local_idx) => {
            if let Some(type_name) = tracker_name {
                compiler.set_local_type_info(local_idx, &type_name);
            }
        }
        BindingInitializerTarget::ModuleBinding(binding_idx) => {
            if let Some(type_name) = tracker_name {
                compiler.set_module_binding_type_info(binding_idx, &type_name);
            }
        }
    }
    true
}

pub(crate) fn record_binding_concrete_fact(
    compiler: &mut BytecodeCompiler,
    target: BindingInitializerTarget,
    concrete_type: ConcreteType,
    source: BindingConcreteFactSource,
) {
    let fact = BindingConcreteFact {
        concrete_type,
        source,
    };
    match target {
        BindingInitializerTarget::Local(local_idx) => {
            compiler
                .current_function_local_concrete_facts
                .insert(local_idx, fact);
        }
        BindingInitializerTarget::ModuleBinding(binding_idx) => {
            compiler
                .module_binding_concrete_facts
                .insert(binding_idx, fact);
        }
    }
}

fn tracker_name_for_array_element(elem: &ConcreteType) -> Option<String> {
    let elem_ann = crate::compiler::expressions::closures::concrete_type_to_type_annotation(elem)?;
    let vec_ann = TypeAnnotation::Generic {
        name: shape_ast::ast::TypePath::simple("Vec"),
        args: vec![elem_ann],
    };
    BytecodeCompiler::tracked_type_name_from_annotation(&vec_ann)
}

/// R3-elemerasure (strict-flip): derive the result `ConcreteType` of a builtin
/// (PHF) array method call from the receiver's proven `ConcreteType`, driven by
/// the method's REGISTERED `GenericMethodSignature` return shape.
///
/// The `Self`-returning array methods (`sort`/`reverse`/`take`/`drop`/`slice`/
/// `skip`/`clone`/`unique`/`flatten`/`distinct`/`sortBy`/`concat`/`groupBy`) and
/// the element-returning ones (`first`/`last`/`pop`/`find`) are NOT
/// monomorphized stdlib functions — `specialized_call_return_concrete_type`
/// finds no call-site record for them, so the concrete element/return type was
/// lost across the chain (`[1,2,3].sort().map(|x| x*x)` saw `x: unknown`).
///
/// Recovery is type-proven, NOT broad-suppression: the receiver's element type
/// must already be known (an `Array<T>` `ConcreteType`), and the result is
/// computed strictly from the method's registered return shape —
/// `TypeParamExpr::SelfType` → the receiver's own `Array<T>`,
/// `TypeParamExpr::ReceiverParam(0)` → the element `T`. Any other return shape
/// (`int`/`bool`/`Vec<MethodParam(0)>`/etc.), an unregistered method, or an
/// unproven receiver yields `None` (the resolver falls back cleanly — no
/// fabrication, no Bool-default; per ADR-006 §2.7.5 the registered signature IS
/// the proof).
pub fn method_call_receiver_derived_concrete_type(
    compiler: &BytecodeCompiler,
    receiver: &Expr,
    method: &str,
) -> Option<ConcreteType> {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_system::Type;
    use shape_runtime::type_system::checking::TypeParamExpr;

    // The receiver's type must be proven.
    let receiver_ct = concrete_type_for_expr(compiler, receiver)?;

    // --- Array<T> receivers: generic-signature-driven element/Self return ---
    if let ConcreteType::Array(elem) = &receiver_ct {
        let elem_ct = elem.as_ref().clone();
        // Drive off the method's REGISTERED return shape — never a hardcoded
        // list. Builtin array methods register under the `"Vec"` receiver name
        // (same key the inference engine resolves against).
        if let Some(sig) = compiler
            .method_table
            .lookup_generic_signature("Vec", method)
        {
            return match &sig.return_type {
                // `Self` → the receiver array type unchanged (sort/reverse/take/…).
                TypeParamExpr::SelfType => Some(receiver_ct),
                // `ReceiverParam(0)` → the element type (first/last/pop/find).
                TypeParamExpr::ReceiverParam(0) => Some(elem_ct),
                _ => None,
            };
        }
        return None;
    }

    // --- ROOT-1 (F2): scalar receiver with a MONOMORPHIC method whose
    // registered return annotation is a fully-concrete shape. The canonical
    // case is `"a,b,c".split(",")` → `Array<string>`: the string `split`
    // method is registered (`method_table.rs` `str_methods`) with a CONCRETE
    // `Array<string>` return, but it is NOT a `"Vec"` generic signature and
    // NOT a monomorphized stdlib function, so neither the Array-receiver path
    // above nor `specialized_call_return_concrete_type` recovered it. The
    // result `ConcreteType` was lost, so `let parts = "..".split(",")` recorded
    // nothing and a downstream `parts[0] + parts[1]` saw `unknown + unknown`.
    //
    // Recover it from the method's REGISTERED concrete return Type (ADR-006
    // §2.7.5 — the registered signature IS the proof). Bounded to scalar
    // receivers whose method-table receiver name is statically known
    // (`string`); the return annotation must convert to a FULLY-concrete
    // `ConcreteType` (no type variable). A method with a generic/var return,
    // an unregistered method, or an unresolvable receiver yields `None` — clean
    // fall-through, no fabrication, no Bool-default. Composes for the chained /
    // nested forms (`m.split(",")[0].toUpperCase()`): `split` → `Array<string>`,
    // index-read unwraps to `string`, `toUpperCase` resolves via the same
    // monomorphic lookup.
    // --- traits (S4): user-defined struct/enum receiver with an `extend` /
    // `impl Trait for T` method whose DECLARED return type is concrete. The
    // method is registered in the same `method_table` keyed by the struct name
    // (`register_extend` → `register_user_method("Point", "sum", [], int)`).
    // Without recovering it, `let a = p.sum()` (un-annotated) records no
    // ConcreteType for `a`, the tracker falls back to the default numeric kind
    // (`number`), and a downstream `a + a` emits `AddNumber` → `28.0` instead of
    // `28` (silent float corruption). The DECLARED return annotation IS the
    // proof (ADR-006 §2.7.5); only a fully-concrete return converts (a generic
    // / type-var return yields `None` — clean fall-through, no fabrication, no
    // Bool-default).
    if let Some(struct_name) =
        crate::compiler::patterns::binding::concrete_type_tracker_name(&receiver_ct)
    {
        // `extend T { method m(...) -> R }` desugars to a function registered
        // as `function_defs["T.m"]` (`desugar_extend_method`,
        // `statements.rs:2109`); an `impl Trait for T` method registers as
        // `function_defs["T::m"]` with its return type back-filled from the
        // trait declaration (`desugar_impl_method`). Read the declared return
        // annotation from whichever desugared function def exists and convert a
        // fully-concrete return — that annotation IS the proof (ADR-006 §2.7.5).
        let candidates = [
            format!("{}.{}", struct_name, method),
            format!("{}::{}", struct_name, method),
        ];
        for fname in &candidates {
            if let Some(fdef) = compiler.function_defs.get(fname) {
                if let Some(ann) = fdef.return_type.as_ref() {
                    if let Some(ct) = concrete_type_from_annotation(ann, &HashMap::new()) {
                        return Some(ct);
                    }
                }
            }
        }
    }

    let receiver_type_name = match &receiver_ct {
        ConcreteType::String => Some("string"),
        _ => None,
    }?;
    let recv_type = Type::Concrete(TypeAnnotation::Basic(receiver_type_name.to_string()));
    let sig = compiler.method_table.lookup(&recv_type, method)?;
    if let Type::Concrete(ann) = &sig.return_type {
        // Only a fully-concrete annotation converts (no type-var leaks).
        return concrete_type_from_annotation(ann, &HashMap::new());
    }
    None
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
        // A char literal IS its integer code point (operators.mdx). Code points
        // range up to U+10FFFF, so they are `int` (i64), not i8.
        Literal::Char(_) => Some(ConcreteType::I64),
        Literal::FormattedString { .. } => Some(ConcreteType::String),
        Literal::Bool(_) => Some(ConcreteType::Bool),
        Literal::None => None,
        Literal::Unit => Some(ConcreteType::Void),
        Literal::Timeframe(_) => None,
    }
}

/// U4-5: public wrapper so the inference ladder (`tracked_array_element_type`)
/// can read an identifier's structural `ConcreteType` instead of re-parsing the
/// stringly tracker `type_name`.
pub(crate) fn identifier_concrete_type_pub(
    compiler: &BytecodeCompiler,
    name: &str,
) -> Option<ConcreteType> {
    identifier_concrete_type(compiler, name)
}

fn identifier_concrete_type(compiler: &BytecodeCompiler, name: &str) -> Option<ConcreteType> {
    // Local slot first.
    if let Some(local_idx) = compiler_resolve_local(compiler, name) {
        let pending_empty_array = local_is_unannotated_empty_array_accumulator(compiler, local_idx);
        if !pending_empty_array {
            if let Some(ct) = binding_fact_capture_type(compiler, name) {
                return Some(ct);
            }
            if compiler_resolve_module_binding(compiler, name).is_none() {
                if let Some(ct) = unique_named_binding_fact_concrete_type(compiler, name) {
                    return Some(ct);
                }
            }
        }
        if let Some(ct) = current_function_param_concrete_type_from_facts(compiler, name, local_idx)
        {
            return Some(ct);
        }
        if let Some(ct) = compiler
            .current_function_local_concrete_facts
            .get(&local_idx)
            .map(|fact| fact.concrete_type.clone())
        {
            return Some(ct);
        }
        if let Some(ct) = compiler
            .type_tracker
            .get_local_type(local_idx)
            .and_then(|info| concrete_type_from_tracker_info(compiler, info))
        {
            return Some(ct);
        }
        return None;
    }

    // Module binding fallback.
    if let Some(binding_idx) = compiler_resolve_module_binding(compiler, name) {
        let pending_empty_array =
            module_binding_is_unannotated_empty_array_accumulator(compiler, binding_idx);
        if !pending_empty_array {
            if let Some(ct) = compiler
                .module_binding_spans
                .get(&binding_idx)
                .copied()
                .and_then(|span| compiler.inference_facts.binding_type(span))
                .and_then(|ty| concrete_type_from_inference_fact(compiler, ty))
            {
                return Some(ct);
            }
        }
        if let Some(ct) = compiler
            .module_binding_concrete_facts
            .get(&binding_idx)
            .map(|fact| fact.concrete_type.clone())
        {
            return Some(ct);
        }
        if let Some(ct) = compiler
            .type_tracker
            .get_binding_type(binding_idx)
            .and_then(|info| concrete_type_from_tracker_info(compiler, info))
        {
            return Some(ct);
        }
    }

    None
}

fn local_is_unannotated_empty_array_accumulator(
    compiler: &BytecodeCompiler,
    local_idx: u16,
) -> bool {
    compiler
        .empty_array_accumulators
        .contains_key(&crate::compiler::EmptyArrayAccumulatorKey::Local(local_idx))
        || compiler
            .current_function_local_concrete_facts
            .get(&local_idx)
            .is_some_and(|fact| {
                matches!(
                    fact.source,
                    BindingConcreteFactSource::EmptyArrayAccumulator
                )
            })
}

fn module_binding_is_unannotated_empty_array_accumulator(
    compiler: &BytecodeCompiler,
    binding_idx: u16,
) -> bool {
    compiler.empty_array_accumulators.contains_key(
        &crate::compiler::EmptyArrayAccumulatorKey::ModuleBinding(binding_idx),
    ) || compiler
        .module_binding_concrete_facts
        .get(&binding_idx)
        .is_some_and(|fact| {
            matches!(
                fact.source,
                BindingConcreteFactSource::EmptyArrayAccumulator
            )
        })
}

fn unique_named_binding_fact_concrete_type(
    compiler: &BytecodeCompiler,
    name: &str,
) -> Option<ConcreteType> {
    let mut matching = compiler
        .inference_facts
        .bindings_named(name)
        .filter_map(|fact| concrete_type_from_inference_fact(compiler, &fact.ty));
    let first = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    Some(first)
}

fn concrete_type_from_tracker_info(
    compiler: &BytecodeCompiler,
    info: &crate::type_tracking::VariableTypeInfo,
) -> Option<ConcreteType> {
    let type_name = info.type_name.as_deref()?;
    concrete_type_from_type_name(Some(type_name))
        .or_else(|| struct_or_enum_concrete_type(compiler, type_name))
}

fn current_function_param_concrete_type_from_facts(
    compiler: &BytecodeCompiler,
    name: &str,
    local_idx: u16,
) -> Option<ConcreteType> {
    let param_idx = compiler
        .current_function_params
        .iter()
        .position(|param| param.simple_name() == Some(name))?;
    if usize::from(local_idx) != param_idx {
        return None;
    }

    let current_fn_idx = compiler.current_function?;
    let current_fn_name = compiler
        .program
        .functions
        .get(current_fn_idx)?
        .name
        .as_str();
    let Type::Function { params, .. } = compiler
        .inference_facts
        .function_signature(current_fn_name)?
        .canonicalize()
    else {
        return None;
    };
    concrete_type_from_inference_fact(compiler, params.get(param_idx)?)
}

/// T1 sub-case (a) (strict-flip, 2026-06-20): map a schema `FieldType` to its
/// `ConcreteType` for the `PropertyAccess` field-result arm of
/// `concrete_type_for_expr`. Scalars + arrays + named-object fields map; a
/// field with no statically-mappable kind (`Any` / `HashMap` / `Set` / `Option`
/// — slot bits are an `Arc<_>` pointer per ADR-006 §2.7.5) yields `None` so the
/// caller falls through cleanly (no fabrication, no `Any` projection).
fn field_type_to_concrete(ft: &shape_runtime::type_schema::FieldType) -> Option<ConcreteType> {
    use shape_runtime::type_schema::FieldType;
    Some(match ft {
        FieldType::I64 => ConcreteType::I64,
        FieldType::F64 => ConcreteType::F64,
        FieldType::Bool => ConcreteType::Bool,
        FieldType::String => ConcreteType::String,
        FieldType::Decimal => ConcreteType::Decimal,
        FieldType::I8 => ConcreteType::I8,
        FieldType::I16 => ConcreteType::I16,
        FieldType::I32 => ConcreteType::I32,
        FieldType::U8 => ConcreteType::U8,
        FieldType::U16 => ConcreteType::U16,
        FieldType::U32 => ConcreteType::U32,
        FieldType::U64 => ConcreteType::U64,
        FieldType::Timestamp => ConcreteType::DateTime,
        FieldType::Array(inner) => ConcreteType::Array(Box::new(field_type_to_concrete(inner)?)),
        FieldType::Object(name) => {
            use shape_value::v2::concrete_type::StructLayoutId;
            ConcreteType::named_struct(name.as_str(), StructLayoutId(0))
        }
        // Pointer-backed / dynamic field kinds have no static scalar
        // projection (ADR-006 §2.7.5) — fall through.
        FieldType::Any | FieldType::Option(_) | FieldType::HashMap { .. } | FieldType::Set(_) => {
            return None;
        }
    })
}

/// Extract a `ConcreteType` from a type tracker type name string.
///
/// Recognises scalar primitive / temporal names (`"int"`, `"number"`,
/// `"string"`, `"bool"`, `"decimal"`, `"DateTime"`, ...). This is the
/// fall-through used when explicit inference facts miss — notably for
/// primitive parameters whose only tracker record is a scalar name.
///
/// U4-5/U4-7: the `"Vec<...>"` array branch is deleted. The array element
/// `ConcreteType` is served STRUCTURALLY by whole-binding
/// `ConcreteType::Array(elem)` entries (which run before this fallback), so the
/// `strip_prefix("Vec<")` re-parse — the read half of the Rep-B string
/// round-trip — is gone. Array tracker names no longer round-trip through a
/// string here.
fn concrete_type_from_type_name(type_name: Option<&str>) -> Option<ConcreteType> {
    let name = type_name?;
    // Scalar types
    match name {
        "int" => Some(ConcreteType::I64),
        "number" => Some(ConcreteType::F64),
        "string" => Some(ConcreteType::String),
        "bool" => Some(ConcreteType::Bool),
        "decimal" => Some(ConcreteType::Decimal),
        // T1 sub-case (c) (strict-flip, 2026-06-20): a `DateTime`-typed binding
        // (notably a fn PARAMETER `d1: DateTime`) records the tracker type-name
        // "DateTime". Recognising it here lets `identifier_concrete_type` prove
        // the receiver of `d1.unix_timestamp()` is a `DateTime`, so the
        // `concrete_type_for_expr` DateTime instance-method arm stamps the
        // method's `-> int` return — the ROOT-2 inline-method fix extended to a
        // param receiver. The tracker name IS the proof (ADR-006 §2.7.5).
        "DateTime" => Some(ConcreteType::DateTime),
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
        TypeParam::Type {
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

    fn unannotated_func_param(name: &str) -> FunctionParameter {
        FunctionParameter {
            pattern: DestructurePattern::Identifier(name.to_string(), Span::default()),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: None,
            default_value: None,
        }
    }

    fn bytecode_function(name: &str, arity: u16) -> crate::bytecode::Function {
        crate::bytecode::Function {
            name: name.to_string(),
            arity,
            param_names: Vec::new(),
            locals_count: arity,
            entry_point: 0,
            body_length: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: Vec::new(),
            ref_mutates: Vec::new(),
            mutable_captures: Vec::new(),
            frame_descriptor: None,
            osr_entry_points: Vec::new(),
            mir_data: None,
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

    fn fact_basic(name: &str) -> Type {
        Type::Concrete(TypeAnnotation::Basic(name.to_string()))
    }

    fn fact_generic(name: &str, args: Vec<Type>) -> Type {
        Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(TypePath::simple(
                name,
            )))),
            args,
        }
    }

    fn install_inference_facts(
        compiler: &mut BytecodeCompiler,
        top_level_types: HashMap<String, Type>,
        binding_facts: HashMap<Span, shape_runtime::type_system::BindingFact>,
    ) {
        compiler.inference_facts = shape_runtime::type_system::InferenceFacts::with_binding_facts(
            top_level_types,
            HashMap::new(),
            binding_facts,
        );
    }

    fn install_binding_fact(compiler: &mut BytecodeCompiler, name: &str, span: Span, ty: Type) {
        let mut binding_facts = HashMap::new();
        binding_facts.insert(
            span,
            shape_runtime::type_system::BindingFact {
                name: name.to_string(),
                binder_span: span,
                initializer_span: None,
                ty,
            },
        );
        install_inference_facts(compiler, HashMap::new(), binding_facts);
    }

    #[test]
    fn binding_fact_capture_type_uses_local_binder_span() {
        let span = Span::new(10, 11);
        let mut compiler = BytecodeCompiler::new();
        compiler.locals = vec![HashMap::new()];
        compiler.locals[0].insert("m".to_string(), 0);
        compiler.local_binding_spans.insert(0, span);
        install_binding_fact(
            &mut compiler,
            "m",
            span,
            fact_generic("HashMap", vec![fact_basic("string"), fact_basic("int")]),
        );

        assert_eq!(
            binding_fact_capture_type(&compiler, "m"),
            Some(ConcreteType::HashMap(
                Box::new(ConcreteType::String),
                Box::new(ConcreteType::I64),
            )),
        );
    }

    #[test]
    fn identifier_concrete_type_prefers_runtime_binding_fact_over_local_fact() {
        let span = Span::new(12, 14);
        let mut compiler = BytecodeCompiler::new();
        compiler.locals = vec![HashMap::new()];
        compiler.locals[0].insert("xs".to_string(), 0);
        compiler.local_binding_spans.insert(0, span);
        record_binding_concrete_fact(
            &mut compiler,
            BindingInitializerTarget::Local(0),
            ConcreteType::Array(Box::new(ConcreteType::String)),
            BindingConcreteFactSource::StructuralInitializer,
        );
        install_binding_fact(
            &mut compiler,
            "xs",
            span,
            fact_generic("Array", vec![fact_basic("int")]),
        );

        assert_eq!(
            identifier_concrete_type_pub(&compiler, "xs"),
            Some(ConcreteType::Array(Box::new(ConcreteType::I64))),
        );
    }

    #[test]
    fn identifier_concrete_type_keeps_local_shadow_when_only_module_has_fact() {
        let module_span = Span::new(30, 35);
        let mut compiler = BytecodeCompiler::new();
        compiler.locals = vec![HashMap::new()];
        compiler.locals[0].insert("xs".to_string(), 0);
        compiler.module_bindings.insert("xs".to_string(), 7);
        compiler.module_binding_spans.insert(7, module_span);
        install_binding_fact(
            &mut compiler,
            "xs",
            module_span,
            fact_generic("Array", vec![fact_basic("int")]),
        );

        assert_eq!(identifier_concrete_type_pub(&compiler, "xs"), None,);
    }

    #[test]
    fn identifier_concrete_type_recovers_param_from_function_signature_fact() {
        let mut compiler = BytecodeCompiler::new();
        compiler.locals = vec![HashMap::new()];
        compiler.locals[0].insert("xs".to_string(), 0);
        compiler.current_function_params = vec![unannotated_func_param("xs")];
        compiler
            .program
            .functions
            .push(bytecode_function("takes_xs", 1));
        compiler.current_function = Some(0);

        let mut top_level_types = HashMap::new();
        top_level_types.insert(
            "takes_xs".to_string(),
            Type::Function {
                params: vec![fact_generic("Array", vec![fact_basic("int")])],
                returns: Box::new(fact_basic("void")),
            },
        );
        install_inference_facts(&mut compiler, top_level_types, HashMap::new());

        assert_eq!(
            identifier_concrete_type_pub(&compiler, "xs"),
            Some(ConcreteType::Array(Box::new(ConcreteType::I64))),
        );
    }

    #[test]
    fn identifier_concrete_type_prefers_runtime_binding_fact_over_module_fact() {
        let module_span = Span::new(40, 45);
        let mut compiler = BytecodeCompiler::new();
        compiler.module_bindings.insert("xs".to_string(), 7);
        compiler.module_binding_spans.insert(7, module_span);
        record_binding_concrete_fact(
            &mut compiler,
            BindingInitializerTarget::ModuleBinding(7),
            ConcreteType::Array(Box::new(ConcreteType::String)),
            BindingConcreteFactSource::StructuralInitializer,
        );
        install_binding_fact(
            &mut compiler,
            "xs",
            module_span,
            fact_generic("Array", vec![fact_basic("int")]),
        );

        assert_eq!(
            identifier_concrete_type_pub(&compiler, "xs"),
            Some(ConcreteType::Array(Box::new(ConcreteType::I64))),
        );
    }

    #[test]
    fn identifier_concrete_type_recovers_unique_binding_fact_without_local_span() {
        let span = Span::new(50, 55);
        let mut compiler = BytecodeCompiler::new();
        compiler.locals = vec![HashMap::new()];
        compiler.locals[0].insert("xs".to_string(), 0);
        install_binding_fact(
            &mut compiler,
            "xs",
            span,
            fact_generic("Array", vec![fact_basic("int")]),
        );

        assert_eq!(
            identifier_concrete_type_pub(&compiler, "xs"),
            Some(ConcreteType::Array(Box::new(ConcreteType::I64))),
        );
    }

    #[test]
    fn identifier_concrete_type_recovers_user_type_from_tracker_name() {
        let mut compiler = BytecodeCompiler::new();
        compiler.locals = vec![HashMap::new()];
        compiler.locals[0].insert("user".to_string(), 0);
        compiler.struct_types.insert(
            "User".to_string(),
            (vec!["score".to_string()], Span::default()),
        );
        compiler.set_local_type_info(0, "User");

        assert!(matches!(
            identifier_concrete_type_pub(&compiler, "user"),
            Some(ConcreteType::Struct(layout)) if layout.name.as_deref() == Some("User")
        ));
    }

    #[test]
    fn identifier_concrete_type_recovers_local_initializer_fact_without_shadow() {
        let mut compiler = BytecodeCompiler::new();
        compiler.locals = vec![HashMap::new()];
        compiler.locals[0].insert("r".to_string(), 0);
        record_binding_concrete_fact(
            &mut compiler,
            BindingInitializerTarget::Local(0),
            ConcreteType::Result(Box::new(ConcreteType::I64), Box::new(ConcreteType::Void)),
            BindingConcreteFactSource::StructuralInitializer,
        );

        assert_eq!(
            identifier_concrete_type_pub(&compiler, "r"),
            Some(ConcreteType::Result(
                Box::new(ConcreteType::I64),
                Box::new(ConcreteType::Void),
            )),
        );
    }

    #[test]
    fn identifier_concrete_type_recovers_module_initializer_fact() {
        let mut compiler = BytecodeCompiler::new();
        compiler.module_bindings.insert("r".to_string(), 3);
        record_binding_concrete_fact(
            &mut compiler,
            BindingInitializerTarget::ModuleBinding(3),
            ConcreteType::Result(Box::new(ConcreteType::I64), Box::new(ConcreteType::Void)),
            BindingConcreteFactSource::StructuralInitializer,
        );

        assert_eq!(
            identifier_concrete_type_pub(&compiler, "r"),
            Some(ConcreteType::Result(
                Box::new(ConcreteType::I64),
                Box::new(ConcreteType::Void),
            )),
        );
    }

    #[test]
    fn identifier_concrete_type_returns_none_without_local_fact() {
        let mut compiler = BytecodeCompiler::new();
        compiler.locals = vec![HashMap::new()];
        compiler.locals[0].insert("r".to_string(), 0);

        assert_eq!(identifier_concrete_type_pub(&compiler, "r"), None);
    }

    #[test]
    fn identifier_concrete_type_returns_none_without_module_fact() {
        let mut compiler = BytecodeCompiler::new();
        compiler.module_bindings.insert("r".to_string(), 3);

        assert_eq!(identifier_concrete_type_pub(&compiler, "r"), None);
    }

    #[test]
    fn binding_fact_capture_type_prefers_local_shadow_over_module_binding() {
        let local_span = Span::new(20, 21);
        let module_span = Span::new(30, 31);
        let mut compiler = BytecodeCompiler::new();
        compiler.locals = vec![HashMap::new()];
        compiler.locals[0].insert("items".to_string(), 0);
        compiler.local_binding_spans.insert(0, local_span);
        compiler.module_bindings.insert("items".to_string(), 7);
        compiler.module_binding_spans.insert(7, module_span);

        let mut binding_facts = HashMap::new();
        binding_facts.insert(
            local_span,
            shape_runtime::type_system::BindingFact {
                name: "items".to_string(),
                binder_span: local_span,
                initializer_span: None,
                ty: fact_generic("HashMap", vec![fact_basic("string"), fact_basic("int")]),
            },
        );
        binding_facts.insert(
            module_span,
            shape_runtime::type_system::BindingFact {
                name: "items".to_string(),
                binder_span: module_span,
                initializer_span: None,
                ty: fact_generic("Deque", vec![fact_basic("int")]),
            },
        );
        compiler.inference_facts = shape_runtime::type_system::InferenceFacts::with_binding_facts(
            HashMap::new(),
            HashMap::new(),
            binding_facts,
        );

        assert_eq!(
            binding_fact_capture_type(&compiler, "items"),
            Some(ConcreteType::HashMap(
                Box::new(ConcreteType::String),
                Box::new(ConcreteType::I64),
            )),
        );
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
            Some(ConcreteType::Function(
                shape_value::v2::concrete_type::FunctionTypeId(0),
            )),
        ];

        let resolution = resolve_call_site_type_args(
            &compiler,
            "map",
            &arg_types,
            &["T".to_string(), "U".to_string()],
        );
        assert!(
            resolution.is_none(),
            "U cannot be inferred from opaque closure"
        );
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
            Some(ConcreteType::Function(
                shape_value::v2::concrete_type::FunctionTypeId(0),
            )),
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
        let resolution =
            resolve_call_site_type_args(&compiler, "nonexistent", &arg_types, &["T".to_string()]);
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
        assert_eq!(
            const_value_mono_segment(&ComptimeConstValue::Bool(true)),
            "bool_true"
        );
        assert_eq!(
            const_value_mono_segment(&ComptimeConstValue::Bool(false)),
            "bool_false"
        );
    }

    #[test]
    fn build_mono_key_with_consts_only_const_args() {
        // No type args, single int const arg → "repeat::int_3"
        let key = build_mono_key_with_consts("repeat", &[], &[ComptimeConstValue::Int(3)]);
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
    // ---- B.3: literal-to-ComptimeConstValue helpers ---------------------

    #[test]
    fn comptime_const_from_int_literal() {
        let e = Expr::Literal(shape_ast::ast::Literal::Int(7), span());
        assert_eq!(
            comptime_const_value_from_literal_expr(&e),
            Some(ComptimeConstValue::Int(7))
        );
    }

    #[test]
    fn comptime_const_from_number_literal() {
        let e = Expr::Literal(shape_ast::ast::Literal::Number(3.25), span());
        assert_eq!(
            comptime_const_value_from_literal_expr(&e),
            Some(ComptimeConstValue::Number(3.25))
        );
    }

    #[test]
    fn comptime_const_from_bool_literal() {
        let e = Expr::Literal(shape_ast::ast::Literal::Bool(true), span());
        assert_eq!(
            comptime_const_value_from_literal_expr(&e),
            Some(ComptimeConstValue::Bool(true))
        );
    }

    #[test]
    fn comptime_const_from_string_literal() {
        let e = Expr::Literal(shape_ast::ast::Literal::String("hi".to_string()), span());
        assert_eq!(
            comptime_const_value_from_literal_expr(&e),
            Some(ComptimeConstValue::String("hi".to_string()))
        );
    }

    #[test]
    fn comptime_const_from_negative_int_literal() {
        // -5 parses as UnaryOp(Neg, Int(5))
        let inner = Expr::Literal(shape_ast::ast::Literal::Int(5), span());
        let e = Expr::UnaryOp {
            op: shape_ast::ast::UnaryOp::Neg,
            operand: Box::new(inner),
            span: span(),
        };
        assert_eq!(
            comptime_const_value_from_literal_expr(&e),
            Some(ComptimeConstValue::Int(-5))
        );
    }

    #[test]
    fn comptime_const_rejects_non_literal() {
        // An identifier is not a literal — callers must error at the call site.
        let e = Expr::Identifier("N".to_string(), span());
        assert_eq!(comptime_const_value_from_literal_expr(&e), None);
    }

    #[test]
    fn split_partitions_const_and_type_params_in_declaration_order() {
        // `fn f<T, const N: int, U, const M: int>(...)` — split into
        // type names [T, U] and const names [N, M], preserving order.
        let params = vec![
            TypeParam::Type {
                name: "T".into(),
                span: Span::default(),
                doc_comment: None,
                default_type: None,
                trait_bounds: Vec::new(),
            },
            TypeParam::Const {
                name: "N".into(),
                span: Span::default(),
                doc_comment: None,
                ty: TypeAnnotation::Basic("int".into()),
                default: Some(Expr::Literal(
                    shape_ast::ast::Literal::Int(3),
                    Span::default(),
                )),
            },
            TypeParam::Type {
                name: "U".into(),
                span: Span::default(),
                doc_comment: None,
                default_type: None,
                trait_bounds: Vec::new(),
            },
            TypeParam::Const {
                name: "M".into(),
                span: Span::default(),
                doc_comment: None,
                ty: TypeAnnotation::Basic("int".into()),
                default: Some(Expr::Literal(
                    shape_ast::ast::Literal::Int(5),
                    Span::default(),
                )),
            },
        ];
        let (types, consts) = split_type_and_const_param_names(&params);
        assert_eq!(types, vec!["T".to_string(), "U".to_string()]);
        assert_eq!(consts, vec!["N".to_string(), "M".to_string()]);
    }

    #[test]
    #[ignore = "blocked on turbofish `::<N>` call-site grammar — default-value route \
                is covered end-to-end by `b5_*` tests in cache.rs"]
    fn const_generic_repeat_n_3_end_to_end() {
        // Turbofish-specific test body, once the grammar adds `fn_name::<3>(...)`:
        //
        //   let source = r#"
        //       fn repeat<const N: int>(x: number) -> Array<number> { ... }
        //       repeat::<3>(1.0)
        //   "#;
        //   let (compiler, _) = compile_and_inspect(source);
        //   assert!(compiler.monomorphization_cache.lookup("repeat::int_3").is_some());
        //
        // B.5 status (Track B close-out): the default-value route
        // (`fn f<const N: int = 4>(...)`) is covered end-to-end by
        // `b5_const_generic_*` tests in `cache.rs` — they parse real Shape
        // source, register the FunctionDef via `register_function`, and
        // drive monomorphization through `ensure_monomorphic_function`.
        //
        // What remains for a turbofish-style end-to-end test:
        //   1. Extend `generic_type` in `shape.pest` (or a new
        //      `call_site_turbofish` rule) to allow `::<3>` after an ident.
        //   2. Wire `try_monomorphize_call_site` in
        //      `expressions/function_calls.rs` to also extract const arg
        //      values via `comptime_const_value_from_literal_expr` and call
        //      `ensure_monomorphic_function_with_consts`.
        //   3. Replace this placeholder with a real assertion.
        unreachable!("placeholder for turbofish-supported const generics");
    }

    // ── v0.3 ε-4 — generic-function-chain regression suite ───────────────
    //
    // Before the fix, `concrete_type_for_expr` returned `None` for an
    // `Expr::FunctionCall` argument, so a chained generic call like
    // `id(id(id(42)))` could not resolve the outer call's type params. The
    // outer call fell back to the *unspecialized* generic template (which
    // has empty, body-skipped bytecode); dispatching onto it hung the VM.
    //
    // The fix adds a `FunctionCall` arm that resolves the callee's return
    // ConcreteType (`function_call_return_concrete_type`). These tests drive
    // the full compile + VM pipeline and assert the chains terminate with
    // the correct value.

    use crate::test_utils::{eval_typed_bool, eval_typed_i64};

    #[test]
    fn eps4_generic_id_single_call() {
        assert_eq!(eval_typed_i64("fn id<T>(x: T) -> T { x }\nid(42)"), 42);
    }

    #[test]
    fn eps4_generic_id_chain_2_deep() {
        assert_eq!(eval_typed_i64("fn id<T>(x: T) -> T { x }\nid(id(42))"), 42);
    }

    #[test]
    fn eps4_generic_id_chain_3_deep() {
        // The exact `type_inference::generic_fn_chain_calls` reproducer
        // shape — this hung the VM before the fix.
        assert_eq!(
            eval_typed_i64("fn id<T>(x: T) -> T { x }\nid(id(id(42)))"),
            42
        );
    }

    #[test]
    fn eps4_generic_id_chain_6_deep() {
        assert_eq!(
            eval_typed_i64("fn id<T>(x: T) -> T { x }\nid(id(id(id(id(id(42))))))"),
            42
        );
    }

    #[test]
    fn eps4_generic_fn_chain_nontrivial_body() {
        // A generic body with intermediate let-bindings (not a bare `x`),
        // chained 3-deep.
        assert_eq!(
            eval_typed_i64(
                "fn box_it<T>(x: T) -> T { let tmp = x; let again = tmp; again }\n\
                 box_it(box_it(box_it(7)))"
            ),
            7
        );
    }

    #[test]
    fn eps4_generic_fn_chain_bool() {
        assert!(eval_typed_bool(
            "fn id<T>(x: T) -> T { x }\nid(id(id(true)))"
        ));
    }

    #[test]
    fn eps4_generic_call_as_arg_to_nongeneric_fn() {
        // A generic-call argument passed to a non-generic callee. The inner
        // generic call must resolve a concrete type so monomorphization
        // succeeds rather than falling onto the hanging template.
        assert_eq!(
            eval_typed_i64(
                "fn id<T>(x: T) -> T { x }\n\
                 fn forty_two() -> int { 42 }\n\
                 id(forty_two())"
            ),
            42
        );
    }

    #[test]
    fn eps4_multi_param_generic_chain() {
        // Multi-type-param generic with chained generic-call arguments.
        assert_eq!(
            eval_typed_i64(
                "fn id<T>(x: T) -> T { x }\n\
                 fn snd<A, B>(a: A, b: B) -> B { b }\n\
                 snd(id(1), id(id(99)))"
            ),
            99
        );
    }

    #[test]
    fn eps4_nested_unannotated_fn_chain_not_regressed() {
        // ε-1 non-regression: transitive callsite type propagation for
        // nested unannotated functions. `quad(double(double(x)))` with
        // unannotated params must still resolve and yield 40.
        assert_eq!(
            eval_typed_i64(
                "fn double(x) { x * 2 }\n\
                 fn quad(x) { double(double(x)) }\n\
                 quad(10)"
            ),
            40
        );
    }

    // ── v0.3 WS-6 — generic free function non-scalar argument suite ──────
    //
    // Before the fix, `concrete_type_for_expr` had no arm for struct-literal
    // / enum-constructor / `Some`/`Ok`/`Err` argument expressions, nor for a
    // variable annotated with a struct / enum / `Option<T>` / `Result<T,E>`
    // type. So a generic free-function call like `id(P { a: 7 })` could not
    // bind its type parameter and was rejected with
    // `error[SEMANTIC]: cannot infer type argument(s) ...` — even though the
    // argument's type is fully statically known.
    //
    // The fix adds the missing `concrete_type_for_expr` arms (struct literal,
    // enum constructor, `Some`/`Ok`/`Err`), records declared-annotation
    // ConcreteTypes for `let`-bindings, and threads the source-level type
    // name through `ConcreteType::Struct` / `Enum` (`NamedTypeId`) so the
    // monomorphized specialization carries the proven concrete type.

    #[test]
    fn ws6_generic_id_struct_literal_arg() {
        // `id(P { a: 7 }).a` — a struct literal passed to a generic
        // free function; the result's field access yields the value.
        assert_eq!(
            eval_typed_i64(
                "type P { a: int }\n\
                 fn id<T>(x: T) -> T { x }\n\
                 id(P { a: 7 }).a"
            ),
            7
        );
    }

    #[test]
    fn ws6_generic_id_two_distinct_structs_distinct_specializations() {
        // `id<T>` called with two different struct types must produce two
        // distinct specializations (`id::struct_P` vs `id::struct_Q`) — the
        // `NamedTypeId` name field is what keys them apart in the mono key.
        assert_eq!(
            eval_typed_i64(
                "type P { a: int }\n\
                 type Q { b: int }\n\
                 fn id<T>(x: T) -> T { x }\n\
                 let p = id(P { a: 7 })\n\
                 let q = id(Q { b: 9 })\n\
                 p.a + q.b"
            ),
            16
        );
    }

    #[test]
    fn ws6_generic_id_some_arg() {
        // `Some(5)` (an `Option<int>` constructor) passed to a generic free
        // function. Unwrapping the result yields the payload.
        assert_eq!(
            eval_typed_i64(
                "fn id<T>(x: T) -> T { x }\n\
                 let s = id(Some(5))\n\
                 match s { Some(v) => v, None => 0 }"
            ),
            5
        );
    }

    #[test]
    fn ws6_generic_id_ok_arg() {
        // `Ok(9)` (a `Result` constructor) passed to a generic free function.
        assert_eq!(
            eval_typed_i64(
                "fn id<T>(x: T) -> T { x }\n\
                 let r = id(Ok(9))\n\
                 match r { Ok(v) => v, Err(e) => 0 }"
            ),
            9
        );
    }

    #[test]
    fn ws6_generic_id_option_variable_arg() {
        // A variable with an explicit `Option<int>` annotation passed to a
        // generic free function. The declared-annotation ConcreteType lets
        // the call site bind the type parameter.
        assert_eq!(
            eval_typed_i64(
                "fn id<T>(x: T) -> T { x }\n\
                 let n: Option<int> = Some(5)\n\
                 let r = id(n)\n\
                 match r { Some(v) => v, None => 0 }"
            ),
            5
        );
    }

    #[test]
    fn ws6_generic_id_result_variable_arg() {
        // A variable with an explicit `Result<int, string>` annotation.
        assert_eq!(
            eval_typed_i64(
                "fn id<T>(x: T) -> T { x }\n\
                 let r: Result<int, string> = Ok(9)\n\
                 let x = id(r)\n\
                 match x { Ok(v) => v, Err(e) => 0 }"
            ),
            9
        );
    }

    #[test]
    fn ws6_generic_id_enum_arg() {
        // A user enum constructor passed to a generic free function.
        assert_eq!(
            eval_typed_i64(
                "enum Color { Red, Green }\n\
                 fn id<T>(x: T) -> T { x }\n\
                 let c = id(Color::Red)\n\
                 match c { Color::Red => 1, Color::Green => 2 }"
            ),
            1
        );
    }

    #[test]
    fn ws6_generic_id_array_literal_arg() {
        // An array literal passed to a generic free function — the element
        // type is inferred structurally from the literal's elements when the
        // span side-table is not yet populated (resolution runs before arg
        // compilation).
        assert_eq!(
            eval_typed_i64(
                "fn id<T>(x: T) -> T { x }\n\
                 let a = id([10, 20, 30])\n\
                 a[1]"
            ),
            20
        );
    }

    #[test]
    fn ws6_generic_struct_arg_passed_to_nongeneric_callee() {
        // The result of a generic call on a struct, fed to a non-generic
        // function — the struct round-trips through the monomorphized
        // specialization with the proven concrete type.
        assert_eq!(
            eval_typed_i64(
                "type P { a: int }\n\
                 fn id<T>(x: T) -> T { x }\n\
                 fn geta(p: P) -> int { p.a }\n\
                 geta(id(P { a: 7 }))"
            ),
            7
        );
    }

    #[test]
    fn ws6_generic_id_bare_none_is_clean_compile_error() {
        // WS-2 non-regression: a genuinely type-ambiguous argument — bare
        // `None` with no annotation or context — must STAY a clean compile
        // error, not be force-resolved. `concrete_type_for_expr` returns
        // `None` for a bare `None` constructor, so `resolve_call_site_type_args`
        // cannot bind the type parameter and the call is rejected.
        let result = crate::test_utils::compile_with_prelude("fn id<T>(x: T) -> T { x }\nid(None)");
        assert!(
            result.is_err(),
            "id(None) with no type context must be a clean compile error"
        );
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("cannot infer type argument"),
            "expected the WS-2 'cannot infer type argument' diagnostic, got: {msg}"
        );
    }

    // ── v0.3 WS-6b GAP A — inferred-type let-bound variable arg ──────────
    //
    // WS-6 recorded a `let`-binding's ConcreteType ONLY when the binding
    // carried an explicit type annotation (`let n: Option<int> = ...`). An
    // *inferred*-type binding `let p = P { a: 7 }` carries no annotation, so
    // a later `id(p)` could not resolve the type argument and was rejected
    // with `cannot infer type argument(s)`. WS-6b extends the recording at
    // the let-binding site to also resolve the binding's ConcreteType
    // structurally from the initializer expression via
    // `concrete_type_for_expr`.

    #[test]
    fn ws6b_inferred_struct_variable_arg() {
        // `let p = P { a: 7 }` (no annotation) then `id(p).a`.
        assert_eq!(
            eval_typed_i64(
                "type P { a: int }\n\
                 fn id<T>(x: T) -> T { x }\n\
                 let p = P { a: 7 }\n\
                 id(p).a"
            ),
            7
        );
    }

    #[test]
    fn ws6b_inferred_enum_variable_arg() {
        // `let c = Color::Red` (inferred) passed to a generic free function.
        assert_eq!(
            eval_typed_i64(
                "enum Color { Red, Green }\n\
                 fn id<T>(x: T) -> T { x }\n\
                 let c = Color::Red\n\
                 match id(c) { Color::Red => 1, Color::Green => 2 }"
            ),
            1
        );
    }

    #[test]
    fn ws6b_inferred_option_variable_arg() {
        // `let n = Some(5)` (inferred `Option<int>`) passed to `id`.
        assert_eq!(
            eval_typed_i64(
                "fn id<T>(x: T) -> T { x }\n\
                 let n = Some(5)\n\
                 match id(n) { Some(v) => v, None => 0 }"
            ),
            5
        );
    }

    #[test]
    fn ws6b_inferred_result_variable_arg() {
        // `let r = Ok(9)` (inferred `Result`) passed to `id`.
        assert_eq!(
            eval_typed_i64(
                "fn id<T>(x: T) -> T { x }\n\
                 let r = Ok(9)\n\
                 match id(r) { Ok(v) => v, Err(e) => 0 }"
            ),
            9
        );
    }

    #[test]
    fn ws6b_inferred_struct_variable_arg_local_scope() {
        // The inferred-binding recording must also fire for a function-local
        // `let` (mirror of the module-binding path).
        assert_eq!(
            eval_typed_i64(
                "type P { a: int }\n\
                 fn id<T>(x: T) -> T { x }\n\
                 fn run() -> int { let p = P { a: 7 }\n id(p).a }\n\
                 run()"
            ),
            7
        );
    }

    #[test]
    fn ws6b_inferred_none_variable_still_clean_error() {
        // GAP A non-regression: an inferred `let n = None` is genuinely
        // type-ambiguous — `concrete_type_for_expr` returns `None`, nothing
        // is recorded, and `id(n)` STAYS a clean compile error.
        let result = crate::test_utils::compile_with_prelude(
            "fn id<T>(x: T) -> T { x }\nlet n = None\nid(n)",
        );
        assert!(
            result.is_err(),
            "id(n) where `n` is an inferred bare None must be a clean compile error"
        );
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("cannot infer type argument"),
            "expected the 'cannot infer type argument' diagnostic, got: {msg}"
        );
    }

    // v0.3.3 B4 (references slice D2): nested-index `m[r][c]` arithmetic +
    // typed-reference-parameter object-field arithmetic.

    #[test]
    fn b4_nested_index_arithmetic_recovers_element_type() {
        // `m[r][c]` recovers the element type through TWO index ops:
        // Array<Array<int>> -> Array<int> -> int, so `m[1][0] + 10` (= 13)
        // type-checks and runs under strict typing.
        assert_eq!(
            crate::test_utils::eval_typed_i64(
                "let m: Array<Array<int>> = [[1,2],[3,4]]\nm[1][0] + 10",
            ),
            13
        );
    }

    #[test]
    fn b4_ref_param_number_field_arithmetic() {
        // `fn shift(p: &Point) { p.x + 1.0 }` — field access through a `&`
        // reference parameter recovers the `number` field type and dispatches
        // the arithmetic on the proven kind. `2.5 + 1.0` = 3.5.
        assert_eq!(
            crate::test_utils::eval_typed_f64(
                "type Point { x: number, y: int }\n\
                 fn shift(p: &Point) -> number { return p.x + 1.0 }\n\
                 let pt = Point { x: 2.5, y: 7 }\n\
                 shift(&pt)",
            ),
            3.5
        );
    }

    #[test]
    fn b4_ref_param_int_field_arithmetic() {
        // The `int`-field sibling: `p.y + 1` on a `&Point` param. `7 + 1` = 8.
        assert_eq!(
            crate::test_utils::eval_typed_i64(
                "type Point { x: number, y: int }\n\
                 fn gety(p: &Point) -> int { return p.y + 1 }\n\
                 let pt = Point { x: 2.5, y: 7 }\n\
                 gety(&pt)",
            ),
            8
        );
    }

    #[test]
    fn b4_mut_ref_param_field_mutate_then_read() {
        // `&mut` field mutate-then-read: `bump(&mut pt)` writes `pt.y` then the
        // caller reads it back. 7 + 2 = 9.
        assert_eq!(
            crate::test_utils::eval_typed_i64(
                "type Point { x: number, y: int }\n\
                 fn bump(p: &mut Point) { p.y = p.y + 2 }\n\
                 let mut pt = Point { x: 2.5, y: 7 }\n\
                 bump(&mut pt)\n\
                 pt.y",
            ),
            9
        );
    }

    #[test]
    fn b4_by_value_param_still_rejects_ref_arg() {
        // Non-regression: passing `&pt` to a genuine by-value parameter is
        // still a B0004 compile error (the typed-ref normalization must not
        // make every `&arg` accepted).
        let result = crate::test_utils::compile_with_prelude(
            "type Point { x: number }\n\
             fn byval(p: Point) -> number { return p.x }\n\
             let pt = Point { x: 1.0 }\n\
             byval(&pt)",
        );
        assert!(
            result.is_err(),
            "passing &pt to a by-value parameter must stay a B0004 error"
        );
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("B0004"),
            "expected the B0004 diagnostic, got: {msg}"
        );
    }
}
