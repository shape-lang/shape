//! Type Inference Engine
//!
//! Implements Hindley-Milner style type inference with extensions for
//! Shape's domain-specific features.
//!
//! ## Bidirectional type checking
//!
//! The engine supports three checking modes (see `bidirectional.rs`):
//!
//! - **Infer** -- purely synthesise a type from the expression structure.
//! - **Check(T)** -- verify the expression against an expected type (hard
//!   constraint, emitted for annotated bindings and return positions).
//! - **Synth(T)** -- synthesise with a hint (soft constraint, used for
//!   closure parameter inference from generic method signatures).
//!
//! When a method call like `arr.map(|x| ...)` is encountered, the engine
//! looks up the `GenericMethodSignature` for the receiver type, extracts
//! the expected closure parameter types, and passes them as `Synth` hints
//! so that `x` receives the array element type without annotation.
//!
//! ## Sub-modules
//!
//! - `access` -- property access, index access, field resolution
//! - `bidirectional` -- `CheckMode` and the `check_expr` entry point
//! - `expressions` -- expression-level inference (literals, calls, closures,
//!   match, if/else, binary/unary ops)
//! - `hoisting` -- optimistic pre-pass that collects property assignments
//!   to widen object types before the main inference walk
//! - `items` -- top-level item inference (functions, types, impls, extends)
//! - `operators` -- binary and unary operator type rules
//! - `statements` -- statement-level inference (let, assignment, return,
//!   for, while, blocks)

mod access;
mod bidirectional;
mod expressions;
mod hoisting;
mod items;
mod operators;
mod statements;

pub use bidirectional::CheckMode;
pub use hoisting::{PropertyAssignment, PropertyAssignmentCollector};

use super::checking::MethodTable;
use super::constraints::ConstraintSolver;
use super::environment::TypeEnvironment;
use super::*;
use shape_ast::ast::{ObjectTypeField, Program, Span, StructTypeDef, TypeAnnotation};
use std::collections::HashMap;

use crate::type_system::semantic::{EnumVariant, SemanticType};
use std::collections::HashSet;

/// Finalized type fact for one source binding.
///
/// The key in `InferenceFacts::binding_facts` is the binder/name span. The
/// duplicated `binder_span` keeps the record self-describing for consumers that
/// collect facts by name.
#[derive(Debug, Clone, PartialEq)]
pub struct BindingFact {
    pub name: String,
    pub binder_span: Span,
    pub initializer_span: Option<Span>,
    pub ty: Type,
}

/// Canonical type-inference facts produced by one best-effort program pass.
///
/// This is the named handoff for downstream compiler consumers that need both
/// top-level inferred signatures and the finalized per-expression span table.
/// Both maps come from the same `infer_program_best_effort` run; constructing
/// this carrier must not trigger a second inference pass.
#[derive(Debug, Clone, Default)]
pub struct InferenceFacts {
    top_level_types: HashMap<String, Type>,
    expression_types: HashMap<Span, Type>,
    binding_facts: HashMap<Span, BindingFact>,
}

impl InferenceFacts {
    pub fn new(
        top_level_types: HashMap<String, Type>,
        expression_types: HashMap<Span, Type>,
    ) -> Self {
        Self {
            top_level_types,
            expression_types,
            binding_facts: HashMap::new(),
        }
    }

    pub fn with_binding_facts(
        top_level_types: HashMap<String, Type>,
        expression_types: HashMap<Span, Type>,
        binding_facts: HashMap<Span, BindingFact>,
    ) -> Self {
        Self {
            top_level_types,
            expression_types,
            binding_facts,
        }
    }

    /// Lookup a finalized expression type by source span.
    pub fn expression_type(&self, span: Span) -> Option<&Type> {
        if span.is_dummy() {
            return None;
        }
        self.expression_types.get(&span)
    }

    /// Lookup a top-level function signature inferred for `name`.
    pub fn function_signature(&self, name: &str) -> Option<&Type> {
        match self.top_level_types.get(name) {
            Some(ty @ Type::Function { .. }) => Some(ty),
            _ => None,
        }
    }

    /// Lookup any top-level inferred type by symbol name.
    pub fn top_level_type(&self, name: &str) -> Option<&Type> {
        self.top_level_types.get(name)
    }

    pub fn top_level_types(&self) -> &HashMap<String, Type> {
        &self.top_level_types
    }

    pub fn expression_types(&self) -> &HashMap<Span, Type> {
        &self.expression_types
    }

    /// Lookup a finalized binding fact by binder/name span.
    pub fn binding_fact(&self, span: Span) -> Option<&BindingFact> {
        if span.is_dummy() {
            return None;
        }
        self.binding_facts.get(&span)
    }

    /// Lookup a finalized binding type by binder/name span.
    pub fn binding_type(&self, span: Span) -> Option<&Type> {
        self.binding_fact(span).map(|fact| &fact.ty)
    }

    pub fn binding_facts(&self) -> &HashMap<Span, BindingFact> {
        &self.binding_facts
    }

    pub fn bindings_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a BindingFact> {
        self.binding_facts
            .values()
            .filter(move |fact| fact.name == name)
    }

    pub fn into_expression_types(self) -> HashMap<Span, Type> {
        self.expression_types
    }

    pub fn into_parts(self) -> (HashMap<String, Type>, HashMap<Span, Type>) {
        (self.top_level_types, self.expression_types)
    }

    pub fn into_parts_with_bindings(
        self,
    ) -> (
        HashMap<String, Type>,
        HashMap<Span, Type>,
        HashMap<Span, BindingFact>,
    ) {
        (
            self.top_level_types,
            self.expression_types,
            self.binding_facts,
        )
    }
}

pub struct TypeInferenceEngine {
    /// Type environment tracking variable types
    pub env: TypeEnvironment,
    /// Per-engine generator for fresh type variables (B4). Replaces the
    /// former process-global `NEXT_TYPEVAR_ID` counter so test runs and
    /// independent inference sessions can't alias each other's IDs.
    pub type_var_gen: crate::type_system::TypeVarGen,
    /// Constraint solver for type constraints. SB-2: the solver's unifier is
    /// the SINGLE substitution store — the engine no longer keeps a parallel
    /// one. All grounding reads `self.solver.unifier()` / writes
    /// `self.solver.unifier_mut()`.
    pub(crate) solver: ConstraintSolver,
    /// Generated constraints
    pub(crate) constraints: Vec<(Type, Type)>,
    /// Best-effort origin spans for generated constraints.
    /// Key format must match solver reporting: `"{:?} ~ {:?}"`.
    pub(crate) constraint_origins: HashMap<String, Span>,
    /// Origin spans for callable symbols whose parameter expectations were
    /// inferred from body constraints (e.g. `c + 1` requires `c` numeric).
    pub(crate) callable_origins_by_name: HashMap<String, Span>,
    /// Origin spans for unknown-property errors keyed by property name.
    /// This avoids string-search fallback diagnostics that can drift into comments.
    pub(crate) unknown_property_origins: HashMap<String, Span>,
    /// Origin spans for undefined-variable errors keyed by variable name.
    pub(crate) undefined_variable_origins: HashMap<String, Span>,
    /// Origin spans for non-exhaustive match errors keyed by enum/union label.
    pub(crate) non_exhaustive_match_origins: HashMap<String, Span>,
    /// Stack tracking fallibility at each function scope level
    /// When `?` operator is used, the current scope becomes fallible
    pub(crate) fallible_scopes: Vec<bool>,
    /// Method table for static method resolution
    pub(crate) method_table: MethodTable,
    /// Observed argument types at call sites for each function.
    /// Used to widen unannotated parameter type variables into unions.
    pub(crate) callsite_param_types: HashMap<String, Vec<Vec<Type>>>,
    /// Instantiated result types at named-function call sites.
    ///
    /// Function schemes are instantiated per call, so the return variable held
    /// by `let x = f(...)` is a fresh call-site instance, not necessarily the
    /// same variable stored in `types["f"]`. Once post-callsite proof resolves
    /// the canonical function return, this map lets finalization constrain all
    /// earlier call-result instances to that static return fact.
    pub(crate) callsite_return_types: HashMap<String, Vec<Type>>,
    /// Source type variables for callable parameters, indexed by parameter
    /// position. `None` means parameter was explicitly annotated.
    pub(crate) callable_param_source_vars: HashMap<String, Vec<Option<TypeVar>>>,
    /// HOF return-type aliasing (the sg2 root). When a function's RETURN value
    /// is the result of invoking one of its own fn-typed params in tail/return
    /// position (`fn apply2(f, x, y) { f(x, y) }` — apply2's return IS f's
    /// return type), records `apply2 -> param-index-of-f`. During
    /// `apply_callsite_unions`, once that fn-typed param resolves to a concrete
    /// `Function { returns: R }`, the function's still-unresolved return var is
    /// substituted with the EXACT proven `R` (int stays int, number stays
    /// number — no defaulting). An unresolved `R` leaves the return a variable,
    /// so the case SURFACEs exactly as before (no fabrication).
    pub(crate) callable_return_from_fn_param: HashMap<String, usize>,
    /// HOF array-return aliasing. For a function whose return is an array literal
    /// whose elements are calls to the same fn-typed parameter
    /// (`fn map_pair(f,a,b){ [f(a), f(b)] }`), records the callable parameter
    /// index so the array element type is the genuine callable return.
    pub(crate) callable_array_return_from_fn_param: HashMap<String, usize>,
    /// Whether each callable parameter has a default value.
    /// Used for compile-time arity validation at call sites.
    pub(crate) callable_param_defaults: HashMap<String, Vec<bool>>,
    /// Parameter indices for each named function whose body imposes a
    /// `Numeric` trait bound on an unannotated parameter (e.g. `x` in
    /// `fn f(x) { x * 2 }`). Recorded by `infer_function` but NOT eagerly
    /// collapsed to `number` — eager collapse severs the call-graph link a
    /// transitively-called function needs to resolve its parameter. After
    /// `apply_callsite_unions` has propagated every concrete callsite type,
    /// `refine_numeric_params_post_callsite` collapses any of these
    /// parameters that are *still* unresolved variables to `number` as a
    /// last-resort default.
    pub(crate) callable_numeric_param_indices: HashMap<String, Vec<usize>>,
    /// ROOT-2 (closure-param defaults to number vs int call-site): source type
    /// variables for `Numeric`-bounded UNANNOTATED CLOSURE parameters (e.g. `x`
    /// in `let f = |x| x * 2`). Recorded by the `Expr::FunctionExpr` arm but
    /// NOT eagerly collapsed to `number` — eager collapse stored the closure as
    /// `(number) -> _`, so a later same-scope call site `f(i)` with `i: int`
    /// failed the §2 numeric lattice as `(number) -> _ !~ (int) -> _`. The
    /// closure's param variable flows unchanged into its stored function type,
    /// so the call-site constraint `func_type ~ (int) -> result` resolves it to
    /// the concrete argument type during `solver.solve`. Only a closure that is
    /// NEVER called leaves its var unresolved; `default_unresolved_closure_numeric_params`
    /// then applies the same last-resort `number` default as the named-function
    /// `refine_numeric_params_post_callsite` path. No int VALUE is widened: an
    /// unresolved var adopts `number` only when no concrete arg ever pins it.
    pub(crate) deferred_closure_numeric_param_vars: std::collections::HashSet<TypeVar>,
    /// Numeric-param source vars of a closure LITERAL that was passed as a value
    /// argument to a USER (non-builtin) function call — i.e. the closure ESCAPES
    /// into a callee that may invoke it (`id(|a,b| a*b)`, `applyx(|a,b| a*b,…)`).
    ///
    /// This is the indirected-callable soundness discriminator. When such a
    /// closure's param is invoked with concrete arguments through a layer the
    /// inference engine CAN thread (the direct `applyx(|a,b| a*b,6,7)` case), the
    /// callsite-union fixpoint resolves the param to the concrete arg type and it
    /// is NEVER reached by `default_unresolved_closure_numeric_params`. When the
    /// callable arrives INDIRECTED (returned from `id`, forwarded through a
    /// 2-level wrapper) the link is severed: the param stays free and reaches the
    /// default. Defaulting such a var to `number` is the recurring unsoundness —
    /// an `int` value (6,7) flows into a `number`-typed slot and `MulNumber`
    /// reads it as an f64 denormal (42.0 instead of 42; arr[r] reads arr[0]).
    /// CLAUDE.md: `int` and `number` do NOT unify and an un-inferable result must
    /// SURFACE, so `default_unresolved_closure_numeric_params` REJECTS for vars in
    /// this set instead of defaulting. A never-called closure (`let f = |x| x*3`)
    /// is NOT in this set (it escapes into no user call that could invoke it) and
    /// keeps the harmless `number` default — no value ever flows through it.
    pub(crate) escaping_closure_numeric_param_vars: std::collections::HashSet<TypeVar>,
    /// S1 (forwarded-closure body-literal proof): for a deferred closure
    /// numeric param var, the type its OWN BODY proves via an `int`/`number`
    /// literal pairing (`|x| x * 2` proves `int` from the bare `2`; `|x| x /
    /// 2.0` proves `number`). When an escaping closure's param is NEVER pinned
    /// by a concrete call site (the forwarded `let mul = |x| x * 2;
    /// use_it(mul)` chain — `apply`/`twice` two-level forwarding the resolver
    /// cannot thread), `default_unresolved_closure_numeric_params` consults
    /// this map BEFORE rejecting: the body literal IS the proof (the closure is
    /// not genuinely polymorphic), so it binds the var to the proven type
    /// instead of SURFACEing. A body with NO literal pairing (`|x| x * x`)
    /// records no hint and is REJECTED — genuinely un-inferable, never
    /// number-defaulted. This NEVER overrides a call-site: a closure the
    /// resolver/solver pins is already concrete (not a `Type::Variable`) at the
    /// default pass and is skipped, so §4 literal-adoption at a real call site
    /// (`Array<number>.map(|x| x / 2)`) is untouched.
    pub(crate) deferred_closure_numeric_param_body_hint: std::collections::HashMap<TypeVar, Type>,
    /// Let-bound closures are stored as schemes and instantiated with fresh
    /// TypeVars when forwarded by identifier. Preserve the original closure's
    /// per-param body-literal proof facts by binding name so the fresh vars
    /// inherit the same strict default/reject behavior.
    pub(crate) deferred_closure_numeric_binding_hints: HashMap<String, Vec<(bool, Option<Type>)>>,
    /// Indirected-callable COMPLETENESS extension (full-inference ruling). Each
    /// entry records a closure LITERAL passed as a value argument to a USER
    /// function call (`applyx(|a,b| a*b,6,7)`, `id(|a,b| a*b)`,
    /// `wrap(|a,b| a*b,6,7)`): the receiving function name, the closure's
    /// argument position, and the closure's own (still-unresolved) param vars.
    ///
    /// A post-inference, pre-default pass (`resolve_indirected_closure_arg_params`)
    /// walks the program AST to FOLLOW the callable through indirection — a
    /// forwarding wrapper hop (`fn wrap(f,x,y){ applyx(f,x,y) }`) or an
    /// identity-laundered `let` binding (`let h = id(|a,b| a*b); applyx(h,6,7)`)
    /// — to the concrete invocation site whose argument types prove the closure's
    /// param types. It pushes `closure_param_var[k] ~ <proven outer arg type>`
    /// so the solver pins the closure (int stays int, number stays number — the
    /// proven type is copied, never defaulted). Any hop the pass CANNOT follow
    /// leaves the var unresolved, so `default_unresolved_closure_numeric_params`
    /// still SURFACEs it (the SoundRoot floor: never number-default an
    /// un-inferable indirected callable). Recorded as `(callee_name, arg_index,
    /// closure_param_vars, full_call_args)`.
    pub(crate) escaping_closure_arg_sites:
        Vec<(String, usize, Vec<TypeVar>, Vec<shape_ast::ast::Expr>)>,
    /// ROOT-B: payload type variables of `Ok`/`Err`/`Some` constructors whose
    /// argument was a bare int LITERAL that DEFERRED to the var instead of
    /// pinning it to `int` (see `constructor_literal_payload_defers_to_var` and
    /// the deferral in `infer_function_call`). Mirrors
    /// `deferred_closure_numeric_param_vars`: the var flows unresolved so a
    /// later carrier (`Result<number>` / `Option<number>` return) can resolve
    /// it; if NOTHING resolves it (`let x = Some(42); x`, x used bare) the
    /// post-solve `default_unresolved_constructor_literal_payload_vars` pass
    /// binds it to `int` — the literal's NATURAL type — so the binding stays
    /// concrete (no `Option<T>` un-pinnable error) and no value is widened.
    pub(crate) deferred_constructor_literal_payload_vars: std::collections::HashSet<TypeVar>,
    /// Deferred return unions for callables where one branch returned an unresolved type variable
    /// and another returned a concrete type (e.g. `return c` and `return "hi"`).
    /// We preserve precision by materializing these unions only after call-site widening.
    pub(crate) pending_return_unions: HashMap<TypeVar, Vec<Type>>,
    /// Mapping from declared callable return vars to the inferred return vars
    /// they were constrained against during inference.
    ///
    /// This lets best-effort callsite widening propagate parameter substitutions
    /// to aliased return vars before solver substitutions are applied.
    pub(crate) return_var_aliases: HashMap<TypeVar, TypeVar>,
    /// Stack of explicit return types collected for the currently inferred
    /// callable bodies (functions and function expressions).
    pub(crate) return_scopes: Vec<Vec<Type>>,
    /// Stack of expression-statement result types collected for callable
    /// bodies. Used to infer implicit return unions for expression-style code.
    pub(crate) implicit_return_scopes: Vec<Vec<Type>>,
    /// Stack of `break <value>` types collected for the currently-inferred
    /// `loop` bodies. A `loop` with at least one value-carrying break is an
    /// expression whose type is the unified type of all break values
    /// (control-flow.mdx "Break with Value"). A value-less `loop` stays Void.
    pub(crate) break_scopes: Vec<Vec<Type>>,
    /// Stack of function-local empty-array accumulator names that are returned
    /// by the current callable body. Only these carriers may bind their
    /// element variable to an unresolved pushed argument; other accumulators
    /// must wait for concrete producer proof.
    pub(crate) empty_grow_return_carrier_scopes: Vec<std::collections::HashSet<String>>,
    /// Numeric-conversion §4 literal adoption (return context). Stack of the
    /// currently-enclosing callables' DECLARED return types (one entry pushed
    /// per callable body with an explicit numeric `-> T` annotation; `None`
    /// otherwise). A bare integer `return <lit>` / tail-expression literal whose
    /// value losslessly fits the top-of-stack numeric return type adopts it
    /// (`fn f() -> number { return 42 }`, match-arm literals in a `-> number`
    /// fn), instead of recording an `int` that the §2 lattice would reject
    /// against `number`.
    pub(crate) expected_return_types: Vec<Option<Type>>,
    /// Struct type definitions keyed by name for generic struct-literal inference.
    pub(crate) struct_type_defs: HashMap<String, StructTypeDef>,
    /// Resolved type parameter substitutions at generic call sites.
    /// Key: (function_name, span_start, span_end)
    /// Value: [(original_param_name, concrete_TypeAnnotation)]
    ///
    /// Populated during `infer_function_call` when all type params of a
    /// polymorphic callee resolve to concrete types. Consumed by the
    /// bytecode compiler to drive monomorphization.
    pub callsite_type_args: HashMap<(String, usize, usize), Vec<(String, TypeAnnotation)>>,
    /// J-CT.1: depth of the current `comptime { ... }` nesting.
    ///
    /// Incremented on entering `Expr::Comptime` / `Expr::ComptimeFor` /
    /// `Item::Comptime` during inference; decremented on exit. The
    /// method-call type-checker (`expressions.rs::Expr::MethodCall`)
    /// rejects calls to `comptime impl`-registered methods when this
    /// counter is zero. A counter (not a bool) keeps nested comptime
    /// blocks correct, but in practice the depth stays at 0 or 1.
    pub(crate) comptime_depth: usize,
    /// Let-gen spec §4 (A-enforced): module-scope un-annotated `let`/`var`/`const`
    /// binding name → (declaration span, init-is-a-function-application). After
    /// constraint solving, a binding whose init is a bare APPLICATION (the
    /// grounding's class-(2) `let x = get_none()`) and whose FINAL inferred type
    /// is still a fully-polymorphic carrier is a compile error demanding an
    /// annotation — mirroring the empty-array `let a: Array<T> = []` remedy. A
    /// direct value binding (`let x = None`, the grounding's class-(3)) is NOT
    /// flagged as an application and is left to compile, matching the language's
    /// established acceptance of pure kind-erased `None`. Annotated bindings and
    /// function symbols are never recorded.
    pub(crate) unannotated_let_binding_origins: HashMap<String, (Span, bool)>,
    /// T1 keystone (strict-flip, 2026-06-22): POST-SOLVE per-expression type
    /// table keyed by source span. Populated DURING inference by `infer_expr`
    /// (the synthesized, pre-substitution type at every expression site,
    /// including function-body locals the module-scope re-run cannot see), then
    /// REWRITTEN at the end of `infer_program_best_effort`: every entry is run
    /// through the final unifier substitution, and any entry whose resolved type
    /// still contains a free `Type::Variable` is DROPPED (no Unknown-default —
    /// an un-inferable expression stays absent so the bytecode-compiler boundary
    /// surfaces a genuine compile error, per CLAUDE.md strict-typing). Read by
    /// `BytecodeCompiler::infer_expr_type` (consulted FIRST, before the
    /// per-context patch ladder) via `resolved_expr_type`.
    pub(crate) expr_type_table: HashMap<Span, Type>,
    /// Post-solve binding facts keyed by binder/name span. Populated with the
    /// pre-solve type from `infer_variable_decl`, then finalized through the
    /// same substitution/drop policy as `expr_type_table`.
    pub(crate) binding_fact_table: HashMap<Span, BindingFact>,
    /// Array/object parameter destructure links created while binding function
    /// parameter patterns. Once the whole parameter resolves to a statically
    /// known array/object/struct type, these links bind child binder variables
    /// to the proven element/field type. This is a compile-time fact path only;
    /// it exists because child binder variables can also carry body-local
    /// constraints (e.g. Numeric), and the solver's bound propagation may leave
    /// the original binder var unfinalized.
    pub(crate) param_destructure_array_element_links: Vec<(Type, TypeVar)>,
    pub(crate) param_destructure_field_links: Vec<(Type, String, TypeVar)>,
}

impl Default for TypeInferenceEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeInferenceEngine {
    fn seed_builtin_callable_defaults(defaults: &mut HashMap<String, Vec<bool>>) {
        // range(n), range(start, end), range(start, end, step)
        defaults.insert("range".to_string(), vec![false, true, true]);
        // round(value) -> int — single-arg only (the legacy 2-arg
        // round(value, decimals) form was dead at runtime; see
        // type_system/environment/mod.rs). No optional 2nd arg.
        defaults.insert("round".to_string(), vec![false]);
    }

    pub fn new() -> Self {
        let mut env = TypeEnvironment::new();

        // Add built-in functions
        env.define_builtin_functions();

        let mut callable_param_defaults = HashMap::new();
        Self::seed_builtin_callable_defaults(&mut callable_param_defaults);

        TypeInferenceEngine {
            env,
            type_var_gen: crate::type_system::TypeVarGen::new(),
            solver: ConstraintSolver::new(),
            constraints: Vec::new(),
            constraint_origins: HashMap::new(),
            callable_origins_by_name: HashMap::new(),
            unknown_property_origins: HashMap::new(),
            undefined_variable_origins: HashMap::new(),
            non_exhaustive_match_origins: HashMap::new(),
            fallible_scopes: Vec::new(),
            method_table: MethodTable::new(),
            callsite_param_types: HashMap::new(),
            callsite_return_types: HashMap::new(),
            callable_param_source_vars: HashMap::new(),
            callable_return_from_fn_param: HashMap::new(),
            callable_array_return_from_fn_param: HashMap::new(),
            callable_param_defaults,
            callable_numeric_param_indices: HashMap::new(),
            deferred_closure_numeric_param_vars: std::collections::HashSet::new(),
            deferred_closure_numeric_param_body_hint: std::collections::HashMap::new(),
            deferred_closure_numeric_binding_hints: HashMap::new(),
            escaping_closure_numeric_param_vars: std::collections::HashSet::new(),
            escaping_closure_arg_sites: Vec::new(),
            deferred_constructor_literal_payload_vars: std::collections::HashSet::new(),
            pending_return_unions: HashMap::new(),
            return_var_aliases: HashMap::new(),
            return_scopes: Vec::new(),
            implicit_return_scopes: Vec::new(),
            break_scopes: Vec::new(),
            empty_grow_return_carrier_scopes: Vec::new(),
            expected_return_types: Vec::new(),
            struct_type_defs: HashMap::new(),
            callsite_type_args: HashMap::new(),
            comptime_depth: 0,
            unannotated_let_binding_origins: HashMap::new(),
            expr_type_table: HashMap::new(),
            binding_fact_table: HashMap::new(),
            param_destructure_array_element_links: Vec::new(),
            param_destructure_field_links: Vec::new(),
        }
    }

    /// T1 keystone: the POST-SOLVE resolved type recorded for the expression at
    /// `span`, if inference proved a concrete (fully-resolved) type for it.
    ///
    /// Returns `None` when no entry exists (the engine never walked that
    /// expression, e.g. a synthetic/desugared node) OR when the entry was
    /// dropped post-solve because it remained a free type variable. The caller
    /// (the bytecode-compiler `infer_expr_type` bridge) treats `None` as
    /// "table miss, fall through to the per-context patch ladder" — never as a
    /// license to default an un-inferable expression.
    pub fn resolved_expr_type(&self, span: Span) -> Option<&Type> {
        if span.is_dummy() {
            return None;
        }
        self.expr_type_table.get(&span)
    }

    /// T1 keystone: take ownership of the finalized per-expression type table,
    /// leaving the engine's table empty. Called by the bytecode compiler after
    /// `infer_program_best_effort` so it can consult the resolved types at the
    /// `infer_expr_type` boundary without re-running inference.
    pub fn take_expr_type_table(&mut self) -> HashMap<Span, Type> {
        std::mem::take(&mut self.expr_type_table)
    }

    pub fn take_binding_fact_table(&mut self) -> HashMap<Span, BindingFact> {
        std::mem::take(&mut self.binding_fact_table)
    }

    /// T1 keystone post-solve finalization: rewrite every recorded expression
    /// type through the final substitution and DROP entries that remain
    /// un-inferable (still a free variable after substitution). Called once,
    /// after the unifier has merged the solver's bindings, near the end of
    /// `infer_program_best_effort` / `check_consumer_against_registered_interface`.
    fn finalize_expr_type_table(&mut self) {
        let resolved: HashMap<Span, Type> = self
            .expr_type_table
            .drain()
            .filter_map(|(span, ty)| {
                let resolved = self.solver.unifier().apply_substitutions(&ty);
                // No Unknown-default: a still-free variable (or a type whose
                // structure still contains a free variable) is un-inferable —
                // drop it so the compiler boundary surfaces the genuine error.
                if Self::type_is_fully_resolved(&resolved) {
                    Some((span, resolved))
                } else {
                    None
                }
            })
            .collect();
        self.expr_type_table = resolved;
    }

    fn finalize_binding_fact_table(&mut self) {
        let resolved: HashMap<Span, BindingFact> = self
            .binding_fact_table
            .drain()
            .filter_map(|(span, mut fact)| {
                if span.is_dummy() {
                    return None;
                }
                let resolved_ty = self.solver.unifier().apply_substitutions(&fact.ty);
                if Self::type_is_fully_resolved(&resolved_ty) {
                    fact.ty = resolved_ty;
                    Some((span, fact))
                } else {
                    None
                }
            })
            .collect();
        self.binding_fact_table = resolved;
    }

    fn record_binding_facts_for_decl(&mut self, decl: &shape_ast::ast::VariableDecl, ty: &Type) {
        let initializer_span = decl
            .value
            .as_ref()
            .map(shape_ast::ast::Spanned::span)
            .filter(|span| !span.is_dummy());

        for (name, binder_span) in decl.pattern.get_bindings() {
            let fact_ty = self
                .env
                .lookup(&name)
                .map(|scheme| scheme.ty.clone())
                .unwrap_or_else(|| ty.clone());
            self.binding_fact_table.insert(
                binder_span,
                BindingFact {
                    name,
                    binder_span,
                    initializer_span,
                    ty: fact_ty,
                },
            );
        }
    }

    fn record_binding_facts_for_match_pattern(
        &mut self,
        pattern: &shape_ast::ast::Pattern,
        scrutinee_span: Span,
    ) {
        let initializer_span = (!scrutinee_span.is_dummy()).then_some(scrutinee_span);

        for (name, binder_span) in pattern.get_bindings() {
            if binder_span.is_dummy() {
                continue;
            }
            let Some(scheme) = self.env.lookup(&name) else {
                continue;
            };
            self.binding_fact_table.insert(
                binder_span,
                BindingFact {
                    name,
                    binder_span,
                    initializer_span,
                    ty: scheme.ty.clone(),
                },
            );
        }
    }

    /// Whether a (post-substitution) type is fully resolved — i.e. contains no
    /// free `Type::Variable` anywhere in its structure. Used to decide whether a
    /// recorded expression type is concrete enough to hand to the compiler. A
    /// type with ANY embedded free variable (e.g. `Array<?>`, `(int) -> ?`) is
    /// NOT fully resolved and is dropped from the table.
    fn type_is_fully_resolved(ty: &Type) -> bool {
        match ty {
            Type::Variable(_) => false,
            // A constrained variable is still a variable awaiting a binding.
            Type::Constrained { .. } => false,
            Type::Concrete(_) => true,
            Type::Function { params, returns } => {
                params.iter().all(Self::type_is_fully_resolved)
                    && Self::type_is_fully_resolved(returns)
            }
            Type::Generic { base, args } => {
                Self::type_is_fully_resolved(base) && args.iter().all(Self::type_is_fully_resolved)
            }
        }
    }

    /// J-CT.1: enter a comptime context (e.g. `comptime { ... }`).
    pub(crate) fn enter_comptime(&mut self) {
        self.comptime_depth = self.comptime_depth.saturating_add(1);
    }

    /// Mark the whole current inference run as already proven comptime.
    ///
    /// This is for compiler-synthesized comptime mini-programs whose caller
    /// has already established they came from an actual `comptime { }` path.
    pub fn set_root_comptime_context(&mut self, enabled: bool) {
        self.comptime_depth = usize::from(enabled);
    }

    /// J-CT.1: exit a comptime context.
    pub(crate) fn exit_comptime(&mut self) {
        self.comptime_depth = self.comptime_depth.saturating_sub(1);
    }

    /// J-CT.1: whether the current expression is being checked inside a
    /// `comptime { ... }` block. Drives runtime-call rejection of methods
    /// registered by `comptime impl` blocks.
    pub(crate) fn in_comptime_context(&self) -> bool {
        self.comptime_depth > 0
    }

    /// Register host-known root-scope bindings before program inference.
    ///
    /// These names come from host configuration (project/frontmatter/extensions)
    /// and prevent false-positive "undefined variable" diagnostics in the shared analyzer.
    pub fn register_known_bindings(&mut self, names: &[String]) {
        for name in names {
            if self.env.lookup(name).is_none() {
                // Known extension/module namespaces are unresolved roots that
                // should allow member access/call constraints without producing
                // undefined-variable or concrete-method-not-found errors.
                let fresh = self.fresh_type_var();
                self.env.define(name, TypeScheme::mono(fresh));
            }
        }
    }

    /// Allocate a fresh type variable from this engine's local counter.
    ///
    /// IDs are scoped to a single inference run, so independent
    /// `TypeInferenceEngine` instances (e.g. one per test) cannot alias.
    pub fn fresh_var(&mut self) -> TypeVar {
        self.type_var_gen.fresh_var()
    }

    /// Allocate a fresh type variable wrapped in `Type::Variable`.
    pub fn fresh_type_var(&mut self) -> Type {
        self.type_var_gen.fresh_type()
    }

    /// T1 sub-case (d) (strict-flip, 2026-06-20): the already-SOLVED type of a
    /// binding `name`, as resolved by the completed inference pass. Reads the
    /// env's stored scheme (the full-program pass left it bound) and applies the
    /// pass's substitutions so any element/field type variables collapse to
    /// their concrete forms. Used by the bytecode compiler to recover an
    /// object-literal array's element field types for a destructuring for-in
    /// over a module-scope binding — a fresh `infer_expr` on the bare name would
    /// error `UndefinedVariable` (empty re-run env). Returns `None` when the
    /// name is not bound (no fabrication).
    pub fn resolved_binding_type(&self, name: &str) -> Option<Type> {
        let scheme = self.env.lookup(name)?;
        Some(self.solver.unifier().apply_substitutions(&scheme.ty))
    }

    /// Push a new function scope for fallibility tracking
    pub(crate) fn push_fallible_scope(&mut self) {
        self.fallible_scopes.push(false);
    }

    /// Pop a function scope and return whether it was fallible
    pub(crate) fn pop_fallible_scope(&mut self) -> bool {
        self.fallible_scopes.pop().unwrap_or(false)
    }

    /// Mark the current function scope as fallible (contains `?` operator)
    pub(crate) fn mark_current_scope_fallible(&mut self) {
        if let Some(last) = self.fallible_scopes.last_mut() {
            *last = true;
        }
    }

    pub(crate) fn push_return_scope(&mut self) {
        self.return_scopes.push(Vec::new());
    }

    pub(crate) fn pop_return_scope(&mut self) -> Vec<Type> {
        self.return_scopes.pop().unwrap_or_default()
    }

    pub(crate) fn record_return_type(&mut self, ty: Type) {
        if let Some(scope_returns) = self.return_scopes.last_mut() {
            scope_returns.push(ty);
        }
    }

    pub(crate) fn push_implicit_return_scope(&mut self) {
        self.implicit_return_scopes.push(Vec::new());
    }

    pub(crate) fn pop_implicit_return_scope(&mut self) -> Vec<Type> {
        self.implicit_return_scopes.pop().unwrap_or_default()
    }

    pub(crate) fn record_implicit_return_type(&mut self, ty: Type) {
        if let Some(scope_returns) = self.implicit_return_scopes.last_mut() {
            scope_returns.push(ty);
        }
    }

    pub(crate) fn push_break_scope(&mut self) {
        self.break_scopes.push(Vec::new());
    }

    pub(crate) fn pop_break_scope(&mut self) -> Vec<Type> {
        self.break_scopes.pop().unwrap_or_default()
    }

    /// Record a `break <value>` type into the innermost enclosing `loop` scope.
    /// Returns `true` if a loop scope was present to record into (i.e. the break
    /// targets a `loop`, not a `for`/`while`), letting the caller distinguish a
    /// loop-break from other break sites.
    pub(crate) fn record_break_type(&mut self, ty: Type) -> bool {
        if let Some(scope_breaks) = self.break_scopes.last_mut() {
            scope_breaks.push(ty);
            true
        } else {
            false
        }
    }

    pub(crate) fn record_pending_return_union(
        &mut self,
        base_var: TypeVar,
        additional_members: impl IntoIterator<Item = Type>,
    ) {
        // Borrow the solver (disjoint from `pending_return_unions`) for the
        // single equality relation so the `&mut self.pending_return_unions`
        // entry borrow and the `&self.solver` probe don't conflict.
        let solver = &self.solver;
        let entry = self
            .pending_return_unions
            .entry(base_var)
            .or_insert_with(Vec::new);
        for member in additional_members {
            if !entry
                .iter()
                .any(|existing| solver.probe_equal(existing, &member))
            {
                entry.push(member);
            }
        }
    }

    fn constraint_key(left: &Type, right: &Type) -> String {
        format!("{:?} ~ {:?}", left, right)
    }

    pub(crate) fn push_constraint_with_origin(&mut self, left: Type, right: Type, origin: Span) {
        if !origin.is_dummy() && !origin.is_empty() {
            let key = Self::constraint_key(&left, &right);
            self.constraint_origins.entry(key).or_insert(origin);
        }
        self.constraints.push((left, right));
    }

    pub fn find_origin_for_unsolved_constraints(
        &self,
        constraints: &[(Type, Type)],
    ) -> Option<Span> {
        constraints
            .iter()
            .filter_map(|(left, right)| {
                self.constraint_origins
                    .get(&Self::constraint_key(left, right))
                    .copied()
            })
            .filter(|span| !span.is_dummy() && !span.is_empty())
            .min_by_key(|span| span.start)
    }

    pub(crate) fn register_callable_origin_for_name(&mut self, name: &str, origin: Span) {
        if origin.is_dummy() || origin.is_empty() {
            return;
        }
        self.callable_origins_by_name
            .insert(name.to_string(), origin);
    }

    pub(crate) fn lookup_callable_origin_for_name(&self, name: &str) -> Option<Span> {
        self.callable_origins_by_name.get(name).copied()
    }

    pub(crate) fn register_unknown_property_origin(&mut self, property: &str, origin: Span) {
        if origin.is_dummy() || origin.is_empty() {
            return;
        }
        self.unknown_property_origins
            .entry(property.to_string())
            .or_insert(origin);
    }

    pub(crate) fn overwrite_unknown_property_origin(&mut self, property: &str, origin: Span) {
        if origin.is_dummy() || origin.is_empty() {
            return;
        }
        self.unknown_property_origins
            .insert(property.to_string(), origin);
    }

    pub(crate) fn lookup_unknown_property_origin(&self, property: &str) -> Option<Span> {
        self.unknown_property_origins.get(property).copied()
    }

    pub(crate) fn register_undefined_variable_origin(&mut self, name: &str, origin: Span) {
        if origin.is_dummy() || origin.is_empty() {
            return;
        }
        self.undefined_variable_origins
            .entry(name.to_string())
            .or_insert(origin);
    }

    pub(crate) fn lookup_undefined_variable_origin(&self, name: &str) -> Option<Span> {
        self.undefined_variable_origins.get(name).copied()
    }

    pub(crate) fn register_non_exhaustive_match_origin(&mut self, enum_name: &str, origin: Span) {
        if origin.is_dummy() || origin.is_empty() {
            return;
        }
        self.non_exhaustive_match_origins
            .entry(enum_name.to_string())
            .or_insert(origin);
    }

    pub(crate) fn lookup_non_exhaustive_match_origin(&self, enum_name: &str) -> Option<Span> {
        self.non_exhaustive_match_origins.get(enum_name).copied()
    }

    pub(crate) fn find_any_constraint_origin(&self) -> Option<Span> {
        self.constraint_origins
            .values()
            .copied()
            .filter(|span| !span.is_dummy() && !span.is_empty())
            .min_by_key(|span| span.start)
    }

    pub(crate) fn is_result_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Generic { base, .. } => matches!(
                base.as_ref(),
                Type::Concrete(ann) if ann.as_type_name_str() == Some("Result")
            ),
            Type::Concrete(TypeAnnotation::Generic { name, .. }) => name == "Result",
            _ => false,
        }
    }

    pub(crate) fn is_option_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Generic { base, .. } => matches!(
                base.as_ref(),
                Type::Concrete(ann) if ann.as_type_name_str() == Some("Option")
            ),
            Type::Concrete(TypeAnnotation::Generic { name, .. }) => name == "Option",
            _ => false,
        }
    }

    /// Extract the success (inner) type `T` of a `Result<T, E>` / `Option<T>`.
    /// Returns `None` for any type that is not a Result/Option carrier.
    pub(crate) fn result_or_option_success_type(&self, ty: &Type) -> Option<Type> {
        match ty {
            Type::Generic { base, args } if !args.is_empty() => match base.as_ref() {
                Type::Concrete(ann)
                    if ann
                        .as_type_name_str()
                        .is_some_and(|n| n == "Result" || n == "Option") =>
                {
                    Some(args[0].clone())
                }
                _ => None,
            },
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if (name == "Result" || name == "Option") && !args.is_empty() =>
            {
                Some(Type::Concrete(args[0].clone()))
            }
            _ => None,
        }
    }

    /// Extract the error type `E` of a `Result<T, E>`. For the single-arg
    /// surface form `Result<T>` (the common case — the error type is the
    /// implicit `AnyError` default that the builtin `Ok`/`Err` schemes also
    /// use), returns `AnyError`. `None` only for non-`Result` carriers (Option
    /// has no error payload). Used by the bidirectional constructor-payload
    /// propagation to feed `Err`'s argument the expected error type — without
    /// the `AnyError` default an `Err(...)` branch of a tail `if` against a
    /// `Result<number>` would fail to extract a payload and the if-branch
    /// carrier threading would not fire (the `Ok(x*2)` sibling branch then
    /// keeps an unconstrained fresh `T` and the strict §2 lattice rejects
    /// `Result<int> !~ Result<number>`).
    pub(crate) fn result_error_type(&self, ty: &Type) -> Option<Type> {
        let any_error = || {
            Type::Concrete(TypeAnnotation::Reference(shape_ast::ast::TypePath::simple(
                "AnyError",
            )))
        };
        match ty {
            Type::Generic { base, args } => match base.as_ref() {
                Type::Concrete(ann) if ann.as_type_name_str() == Some("Result") => {
                    if args.len() >= 2 {
                        Some(args[1].clone())
                    } else if args.len() == 1 {
                        Some(any_error())
                    } else {
                        None
                    }
                }
                _ => None,
            },
            Type::Concrete(TypeAnnotation::Generic { name, args }) if name == "Result" => {
                if args.len() >= 2 {
                    Some(Type::Concrete(args[1].clone()))
                } else if args.len() == 1 {
                    Some(any_error())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Push the constraint linking an inferred body-return type to a declared
    /// return annotation, modelling Shape's implicit `Ok`/`Some`-wrap of a bare
    /// return value inside a fallible/optional function.
    ///
    /// When `declared` is `Result<T, E>` / `Option<T>` and `inferred` is NOT
    /// itself a Result/Option, the body is constraining against the success
    /// type `T` (the function will implicitly wrap the value), so we push
    /// `inferred ~ T` instead of `inferred ~ declared`. When `inferred` IS
    /// already a Result/Option the body produced the wrapper directly, so the
    /// direct `inferred ~ declared` constraint is kept. A value whose inner
    /// type genuinely mismatches `T` still rejects via `inferred ~ T`.
    pub(crate) fn push_return_constraint(&mut self, inferred: Type, declared: Type) {
        // `-> any` is an explicit static top-type return contract. Accept the
        // body type proven by inference without turning `any` into a general
        // equality sink for assignments, fields, or unrelated constraints.
        if self.is_any_type(&declared) {
            return;
        }

        let declared_is_result_or_option =
            self.is_result_type(&declared) || self.is_option_type(&declared);
        let inferred_is_result_or_option =
            self.is_result_type(&inferred) || self.is_option_type(&inferred);

        if declared_is_result_or_option && !inferred_is_result_or_option {
            if let Some(success) = self.result_or_option_success_type(&declared) {
                self.constraints.push((inferred, success));
                return;
            }
        }
        self.constraints.push((inferred, declared));
    }

    pub(crate) fn wrap_result_type(&self, inner: Type) -> Type {
        self.wrap_result_type_with_error(inner, self.any_error_type())
    }

    pub(crate) fn wrap_result_type_with_error(&self, inner: Type, err: Type) -> Type {
        Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Result".into()))),
            args: vec![inner, err],
        }
    }

    pub(crate) fn any_error_type(&self) -> Type {
        Type::Concrete(TypeAnnotation::Reference("AnyError".into()))
    }

    pub(crate) fn apply_fallibility_to_return_type(
        &self,
        return_ty: Type,
        is_fallible: bool,
    ) -> Type {
        // A function that uses `?` propagates through a Result OR an Option
        // carrier. When the return type is ALREADY one of those carriers
        // (e.g. a declared `-> Option<int>` or a body that already returns a
        // `Result<T>`), it must NOT be re-wrapped. Re-wrapping an
        // `Option<int>` into `Result<Option<int>>` corrupts the function's
        // type identity: a downstream `match h() { Some(v) => … }` then sees a
        // `Result` scrutinee and the variant-ownership check spuriously
        // rejects the valid `Some` pattern (R1 false positive).
        if is_fallible && !self.is_result_type(&return_ty) && !self.is_option_type(&return_ty) {
            self.wrap_result_type(return_ty)
        } else {
            return_ty
        }
    }

    /// Check if we're inside a function scope
    #[cfg(test)]
    pub(crate) fn in_function_scope(&self) -> bool {
        !self.fallible_scopes.is_empty()
    }

    pub(crate) fn record_function_callsite(&mut self, function_name: &str, arg_types: &[Type]) {
        let entry = self
            .callsite_param_types
            .entry(function_name.to_string())
            .or_insert_with(|| vec![Vec::new(); arg_types.len()]);

        if entry.len() < arg_types.len() {
            entry.resize_with(arg_types.len(), Vec::new);
        }

        for (index, arg_type) in arg_types.iter().enumerate() {
            entry[index].push(arg_type.clone());
        }
    }

    pub(crate) fn record_function_callsite_return(
        &mut self,
        function_name: &str,
        return_type: Type,
    ) {
        self.callsite_return_types
            .entry(function_name.to_string())
            .or_default()
            .push(return_type);
    }

    fn propagate_resolved_call_returns_to_callsite_instances(
        &mut self,
        types: &HashMap<String, Type>,
    ) -> Vec<TypeError> {
        let mut constraints = Vec::new();
        for (function_name, call_returns) in &self.callsite_return_types {
            let Some(Type::Function { returns, .. }) = types.get(function_name) else {
                continue;
            };
            let resolved_return = self.solver.unifier().apply_substitutions(returns.as_ref());
            if !Self::type_is_fully_resolved(&resolved_return) {
                continue;
            }
            for call_return in call_returns {
                constraints.push((call_return.clone(), resolved_return.clone()));
            }
        }

        if constraints.is_empty() {
            return Vec::new();
        }

        match self.solver.solve(&mut constraints) {
            Ok(()) => Vec::new(),
            Err(err) => vec![err],
        }
    }

    /// Refine callable parameter types from constraints generated while inferring
    /// the callable body. This prevents unresolved `unknown` parameter types in
    /// hot paths like closure literals used in arithmetic/object access.
    ///
    /// Returns the parameter indices whose body imposes a `Numeric` trait
    /// bound.
    ///
    /// `is_closure` selects between the two callable kinds:
    ///
    /// * **Closures (`true`)** are NOT part of the callsite-union scheme — a
    ///   closure literal has no named-function symbol-table entry that
    ///   `apply_callsite_unions` could widen. So a closure parameter must be
    ///   pinned down here, from the body's own constraints: a `Numeric`-bounded
    ///   parameter is eagerly collapsed to `number`, and a parameter whose body
    ///   only does field access is eagerly projected to a partial structural
    ///   `Object` type (`project_object_param_fields_from_constraints`).
    ///
    /// * **Named functions (`false`)** ARE part of the callsite-union scheme.
    ///   Their parameters are left as `Type::Variable`s so the call-graph
    ///   fixpoint in `apply_callsite_unions` can widen them to the union of
    ///   the concrete argument types observed at every call site —
    ///   `refine_numeric_params_post_callsite` then applies the deferred
    ///   `number` default to whatever is still a variable.
    ///
    ///   WS-9b: the object projection is ALSO deferred for named functions.
    ///   Eagerly projecting `fn ov(a, b) { a.lo <= b.hi }` to a partial
    ///   `Object([{ lo: unknown }])` parameter severs the parameter variable
    ///   from the call site exactly the way the numeric collapse does — the
    ///   call site's concrete argument is a *named* struct (`Box`), which a
    ///   partial structural `Object` type cannot unify with, so the leftover
    ///   `Object(...) ~ Box` constraint fails as `UnsolvedConstraints` and the
    ///   whole program is spuriously rejected. Keeping the parameter a variable
    ///   lets callsite union resolve it to `Box`, and the body's `HasField`
    ///   constraint then validates against the resolved struct schema.
    pub(crate) fn refine_callable_param_types_from_local_constraints(
        &self,
        param_types: &mut [Type],
        local_constraints: &[(Type, Type)],
        is_closure: bool,
    ) -> Vec<usize> {
        let mut numeric_indices = Vec::new();
        for (index, param_type) in param_types.iter_mut().enumerate() {
            let Type::Variable(param_var) = param_type else {
                continue;
            };

            // Object projection is a closure-only refinement: named functions
            // resolve object parameters through callsite union (see doc above).
            if is_closure {
                if let Some(fields) =
                    self.project_object_param_fields_from_constraints(param_var, local_constraints)
                {
                    *param_type = Type::Concrete(TypeAnnotation::Object(fields));
                    continue;
                }
            }

            if self.var_has_constraint(local_constraints, param_var, |constraint| {
                matches!(constraint, TypeConstraint::ImplementsTrait { trait_name } if trait_name == "Numeric")
            }) {
                numeric_indices.push(index);
                // ROOT-2: closures used to be eagerly collapsed to `number`
                // here. That stored `let f = |x| x * 2` as `(number) -> _`, so a
                // later same-scope call `f(i)` with `i: int` failed the strict
                // §2 numeric lattice (`(number) -> _ !~ (int) -> _`). Leave the
                // param a `Type::Variable` so the call-site constraint
                // (`func_type ~ (int) -> result`, pushed by `infer_function_call`)
                // resolves it to the concrete argument type during
                // `solver.solve`. A never-called closure leaves its var
                // unresolved; the caller records it (via the returned index +
                // `deferred_closure_numeric_param_vars`) so the last-resort
                // `number` default is applied post-solve — the same deferred
                // default the named-function path uses. No int VALUE is widened.
                let _ = is_closure;
            }
        }
        numeric_indices
    }

    pub(crate) fn register_return_var_alias(
        &mut self,
        declared_return_var: TypeVar,
        inferred_return_var: TypeVar,
    ) {
        if declared_return_var != inferred_return_var {
            self.return_var_aliases
                .insert(declared_return_var, inferred_return_var);
        }
    }

    pub(crate) fn find_origin_for_callable_param_constraints(
        &self,
        param_vars: &[TypeVar],
        constraints: &[(Type, Type)],
    ) -> Option<Span> {
        if param_vars.is_empty() {
            return None;
        }

        let target_vars: HashSet<TypeVar> = param_vars.iter().cloned().collect();

        constraints
            .iter()
            .filter(|(left, right)| {
                Self::type_mentions_any_var(left, &target_vars)
                    || Self::type_mentions_any_var(right, &target_vars)
            })
            .filter_map(|(left, right)| {
                self.constraint_origins
                    .get(&Self::constraint_key(left, right))
                    .copied()
            })
            .filter(|span| !span.is_dummy() && !span.is_empty())
            .min_by_key(|span| span.start)
    }

    fn type_mentions_any_var(ty: &Type, vars: &HashSet<TypeVar>) -> bool {
        match ty {
            Type::Variable(var) => vars.contains(var),
            Type::Constrained { var, .. } => vars.contains(var),
            Type::Generic { base, args } => {
                Self::type_mentions_any_var(base, vars)
                    || args
                        .iter()
                        .any(|arg| Self::type_mentions_any_var(arg, vars))
            }
            Type::Function { params, returns } => {
                params
                    .iter()
                    .any(|param| Self::type_mentions_any_var(param, vars))
                    || Self::type_mentions_any_var(returns, vars)
            }
            Type::Concrete(_) => false,
        }
    }

    fn project_object_param_fields_from_constraints(
        &self,
        param_var: &TypeVar,
        local_constraints: &[(Type, Type)],
    ) -> Option<Vec<ObjectTypeField>> {
        let mut fields: Vec<ObjectTypeField> = Vec::new();

        for (lhs, rhs) in local_constraints {
            let Some((var, constraint)) = Self::extract_var_constraint_pair(lhs, rhs) else {
                continue;
            };
            if var != param_var {
                continue;
            }

            if let TypeConstraint::HasField(field_name, expected_ty) = constraint {
                let field_annotation = self
                    .resolve_expected_annotation_from_constraints(expected_ty, local_constraints)
                    .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string()));
                if fields.iter().all(|field| field.name != *field_name) {
                    fields.push(ObjectTypeField {
                        name: field_name.clone(),
                        optional: false,
                        type_annotation: field_annotation,
                        annotations: vec![],
                    });
                }
            }
        }

        if fields.is_empty() {
            None
        } else {
            Some(fields)
        }
    }

    fn resolve_expected_annotation_from_constraints(
        &self,
        expected_ty: &Type,
        local_constraints: &[(Type, Type)],
    ) -> Option<TypeAnnotation> {
        if let Some(annotation) = expected_ty.to_annotation() {
            return Some(annotation);
        }

        let Type::Variable(var) = expected_ty else {
            return None;
        };

        if let Some(annotation) = self.find_concrete_annotation_for_var(local_constraints, var) {
            return Some(annotation);
        }

        if self.var_has_constraint(local_constraints, var, |constraint| {
            matches!(constraint, TypeConstraint::ImplementsTrait { trait_name } if trait_name == "Numeric")
        }) {
            return Some(TypeAnnotation::Basic("number".to_string()));
        }

        None
    }

    fn find_concrete_annotation_for_var(
        &self,
        local_constraints: &[(Type, Type)],
        target: &TypeVar,
    ) -> Option<TypeAnnotation> {
        for (lhs, rhs) in local_constraints {
            if let Type::Variable(var) = lhs {
                if var == target {
                    if let Some(annotation) = rhs.to_annotation() {
                        return Some(annotation);
                    }
                }
            }
            if let Type::Variable(var) = rhs {
                if var == target {
                    if let Some(annotation) = lhs.to_annotation() {
                        return Some(annotation);
                    }
                }
            }
        }

        None
    }

    fn var_has_constraint<F>(
        &self,
        local_constraints: &[(Type, Type)],
        target: &TypeVar,
        predicate: F,
    ) -> bool
    where
        F: Fn(&TypeConstraint) -> bool,
    {
        local_constraints.iter().any(|(lhs, rhs)| {
            Self::extract_var_constraint_pair(lhs, rhs)
                .map(|(var, constraint)| var == target && predicate(constraint))
                .unwrap_or(false)
        })
    }

    fn extract_var_constraint_pair<'a>(
        lhs: &'a Type,
        rhs: &'a Type,
    ) -> Option<(&'a TypeVar, &'a TypeConstraint)> {
        match (lhs, rhs) {
            (Type::Variable(var), Type::Constrained { constraint, .. }) => Some((var, constraint)),
            (Type::Constrained { constraint, .. }, Type::Variable(var)) => Some((var, constraint)),
            _ => None,
        }
    }

    /// Resolve a Named type to a full Enum type if the name refers to an enum
    ///
    /// This is used for exhaustiveness checking - we need the full enum variant
    /// information to verify all cases are covered.
    pub fn resolve_named_to_enum(&self, ty: &SemanticType) -> SemanticType {
        if let SemanticType::Named(name) = ty {
            if let Some(enum_def) = self.env.get_enum(name) {
                let variants = enum_def
                    .members
                    .iter()
                    .map(|m| {
                        use shape_ast::ast::EnumMemberKind;
                        let payload = match &m.kind {
                            EnumMemberKind::Unit { .. } => None,
                            EnumMemberKind::Tuple(types) => {
                                // Convert tuple payload to SemanticType
                                if types.len() == 1 {
                                    Some(annotation_to_semantic(&types[0]))
                                } else {
                                    // Multiple types -> tuple struct
                                    let fields: Vec<_> = types
                                        .iter()
                                        .enumerate()
                                        .map(|(i, t)| {
                                            (format!("_{}", i), annotation_to_semantic(t))
                                        })
                                        .collect();
                                    Some(SemanticType::Struct {
                                        name: format!("{}_{}", name, m.name),
                                        fields,
                                    })
                                }
                            }
                            EnumMemberKind::Struct(fields) => {
                                // Convert struct payload to SemanticType
                                let semantic_fields: Vec<_> = fields
                                    .iter()
                                    .map(|f| {
                                        (f.name.clone(), annotation_to_semantic(&f.type_annotation))
                                    })
                                    .collect();
                                Some(SemanticType::Struct {
                                    name: format!("{}_{}", name, m.name),
                                    fields: semantic_fields,
                                })
                            }
                        };
                        EnumVariant {
                            name: m.name.clone(),
                            payload,
                        }
                    })
                    .collect();
                return SemanticType::Enum {
                    name: name.clone(),
                    variants,
                    type_params: vec![],
                };
            }
        }
        ty.clone()
    }

    /// Check if all types in a list are equal
    ///
    /// Used for match type inference - if all arms return the same type,
    /// use that type; otherwise create a union.
    pub(crate) fn all_types_equal(&self, types: &[Type]) -> bool {
        if types.is_empty() {
            return true;
        }

        let first = &types[0];
        types.iter().all(|t| self.types_equal(first, t))
    }

    /// The SINGLE type-equivalence relation (U1).
    ///
    /// Routes to `ConstraintSolver::probe_equal` — `solve_constraint` run in
    /// non-committing probe mode against the live substitution store. The
    /// standalone structural `types_equal` free fn and `Unifier::try_unify` were
    /// deleted; this is the one relation, so it sees through every encoding
    /// (canonical `Generic{Array}` vs annotation `Concrete(Array)`), the bound
    /// substitution chain, the numeric lattice, and `AnyError`.
    fn types_equal(&self, a: &Type, b: &Type) -> bool {
        self.solver.probe_equal(a, b)
    }

    /// Create a nominal union type from heterogeneous types
    ///
    /// Generates a union type with an auto-generated brand name.
    /// Example: [boolean, string] → Union_boolean_string
    pub(crate) fn create_nominal_union(&mut self, types: &[Type]) -> TypeResult<Type> {
        use shape_ast::ast::TypeAnnotation;

        // Flatten nested unions and deduplicate (preserve first occurrence).
        let mut unique_types = Vec::new();
        for ty in types {
            let mut candidates = Vec::new();
            match ty {
                Type::Concrete(TypeAnnotation::Union(variants)) => {
                    for variant in variants {
                        candidates.push(Type::Concrete(variant.clone()));
                    }
                }
                _ => candidates.push(ty.clone()),
            }

            for candidate in candidates {
                if !unique_types
                    .iter()
                    .any(|existing| self.types_equal(existing, &candidate))
                {
                    unique_types.push(candidate);
                }
            }
        }

        // If only one unique type remains after dedup, return it directly
        if unique_types.len() == 1 {
            return Ok(unique_types.into_iter().next().unwrap());
        }

        // Generate stable brand name from deduplicated type names
        let union_name = self.generate_union_type_name(&unique_types);

        // Create union type annotation from deduplicated types
        let union_annotation = TypeAnnotation::Union(
            unique_types
                .iter()
                .filter_map(|t| self.type_to_annotation(t))
                .collect(),
        );

        // Register as a new nominal type in the environment
        self.register_inferred_union_type(union_name.clone(), union_annotation.clone())?;

        // Return the union annotation directly for proper display
        Ok(Type::Concrete(union_annotation))
    }

    /// Generate a stable union type name from component types
    ///
    /// Example: [bool, string, number] → "Union_bool_string_number"
    fn generate_union_type_name(&self, types: &[Type]) -> String {
        let type_names: Vec<String> = types.iter().map(|t| self.type_name_for_union(t)).collect();
        format!("Union_{}", type_names.join("_"))
    }

    /// Get a simple name for a type (for union naming)
    fn type_name_for_union(&self, ty: &Type) -> String {
        match ty {
            Type::Concrete(ann) => self.annotation_name(ann),
            Type::Variable(_) => "unknown".to_string(),
            Type::Generic { .. } => "generic".to_string(),
            Type::Constrained { .. } => "constrained".to_string(),
            Type::Function { .. } => "function".to_string(),
        }
    }

    /// Get a simple name from a type annotation
    fn annotation_name(&self, ann: &shape_ast::ast::TypeAnnotation) -> String {
        use shape_ast::ast::TypeAnnotation;
        match ann {
            TypeAnnotation::Basic(name) => name.clone(),
            TypeAnnotation::Reference(name) => name.to_string(),
            TypeAnnotation::Borrow { mutable, inner } => {
                if *mutable {
                    format!("&mut {}", self.annotation_name(inner))
                } else {
                    format!("&{}", self.annotation_name(inner))
                }
            }
            TypeAnnotation::Array(_) => "array".to_string(),
            TypeAnnotation::Object(_) => "object".to_string(),
            TypeAnnotation::Function { .. } => "function".to_string(),
            TypeAnnotation::Union(_) => "union".to_string(),
            TypeAnnotation::Tuple(_) => "tuple".to_string(),
            TypeAnnotation::Intersection(_) => "intersection".to_string(),
            TypeAnnotation::Generic { .. } => "generic".to_string(),
            TypeAnnotation::Void => "void".to_string(),
            TypeAnnotation::Never => "never".to_string(),
            TypeAnnotation::Null => "None".to_string(),
            TypeAnnotation::Undefined => "undefined".to_string(),
            TypeAnnotation::Dyn(traits) => format!("dyn {}", traits.join(" + ")),
        }
    }

    /// Convert a Type to TypeAnnotation
    fn type_to_annotation(&self, ty: &Type) -> Option<shape_ast::ast::TypeAnnotation> {
        match ty {
            Type::Concrete(ann) => Some(ann.clone()),
            Type::Variable(_) => None,
            Type::Generic { .. } => ty.to_annotation(),
            Type::Constrained { .. } => None,
            Type::Function { .. } => ty.to_annotation(),
        }
    }

    /// Register an inferred union type in the environment
    fn register_inferred_union_type(
        &mut self,
        name: String,
        union: shape_ast::ast::TypeAnnotation,
    ) -> TypeResult<()> {
        // Register as a type alias in the environment (no meta param overrides for inferred unions)
        self.env.define_type_alias(&name, &union, None);
        Ok(())
    }

    pub(crate) fn is_void_type(&self, ty: &Type) -> bool {
        matches!(ty, Type::Concrete(TypeAnnotation::Void))
    }

    pub(crate) fn is_any_type(&self, ty: &Type) -> bool {
        matches!(ty, Type::Concrete(ann) if ann.as_type_name_str() == Some("any"))
    }

    pub(crate) fn type_contains_unresolved_vars(&self, ty: &Type) -> bool {
        match ty {
            Type::Variable(_) | Type::Constrained { .. } => true,
            Type::Generic { base, args } => {
                self.type_contains_unresolved_vars(base)
                    || args
                        .iter()
                        .any(|arg| self.type_contains_unresolved_vars(arg))
            }
            Type::Function { params, returns } => {
                params
                    .iter()
                    .any(|param| self.type_contains_unresolved_vars(param))
                    || self.type_contains_unresolved_vars(returns)
            }
            Type::Concrete(_) => false,
        }
    }

    pub(crate) fn collect_type_vars(&self, ty: &Type, out: &mut HashSet<TypeVar>) {
        match ty {
            Type::Variable(var) => {
                out.insert(var.clone());
            }
            Type::Constrained { var, .. } => {
                out.insert(var.clone());
            }
            Type::Generic { base, args } => {
                self.collect_type_vars(base, out);
                for arg in args {
                    self.collect_type_vars(arg, out);
                }
            }
            Type::Function { params, returns } => {
                for param in params {
                    self.collect_type_vars(param, out);
                }
                self.collect_type_vars(returns, out);
            }
            Type::Concrete(_) => {}
        }
    }

    fn ensure_no_unresolved_generic_args(&self, ty: &Type) -> TypeResult<()> {
        match ty {
            Type::Generic { base, args } => {
                if args
                    .iter()
                    .any(|arg| self.type_contains_unresolved_vars(arg))
                {
                    let base_name = match base.as_ref() {
                        Type::Concrete(ann) if ann.as_type_name_str().is_some() => {
                            ann.as_type_name_str().unwrap().to_string()
                        }
                        _ => "generic".to_string(),
                    };
                    return Err(TypeError::GenericTypeError {
                        message: format!(
                            "Could not infer generic type arguments for '{}'",
                            base_name
                        ),
                        symbol: Some(base_name),
                    });
                }
                for arg in args {
                    self.ensure_no_unresolved_generic_args(arg)?;
                }
                Ok(())
            }
            Type::Function { params, returns } => {
                for param in params {
                    self.ensure_no_unresolved_generic_args(param)?;
                }
                self.ensure_no_unresolved_generic_args(returns)
            }
            _ => Ok(()),
        }
    }

    fn as_generic_components(&mut self, ty: &Type) -> Option<(Type, Vec<Type>)> {
        match ty {
            Type::Generic { base, args } => {
                let mut normalized_args = args.clone();
                if matches!(
                    base.as_ref(),
                    Type::Concrete(ann) if ann.as_type_name_str() == Some("Result")
                ) && normalized_args.len() == 1
                {
                    normalized_args.push(self.fresh_type_var());
                }
                Some(((*base.clone()), normalized_args))
            }
            Type::Concrete(TypeAnnotation::Generic { name, args }) => {
                let mut normalized_args = args
                    .iter()
                    .map(|arg| Type::Concrete(arg.clone()))
                    .collect::<Vec<_>>();
                if name == "Result" && normalized_args.len() == 1 {
                    normalized_args.push(self.fresh_type_var());
                }
                Some((
                    Type::Concrete(TypeAnnotation::Reference(name.clone())),
                    normalized_args,
                ))
            }
            _ => None,
        }
    }

    fn merge_homogeneous_generic_types(
        &mut self,
        types: &[Type],
        allow_unresolved_generic_args: bool,
    ) -> TypeResult<Option<Type>> {
        if types.is_empty() {
            return Ok(None);
        }

        let Some((base, args)) = self.as_generic_components(&types[0]) else {
            return Ok(None);
        };

        let arity = args.len();
        let mut all_args: Vec<Vec<Type>> = vec![Vec::new(); arity];

        for ty in types {
            let Some((candidate_base, candidate_args)) = self.as_generic_components(ty) else {
                return Ok(None);
            };
            if !self.types_equal(&base, &candidate_base) || candidate_args.len() != arity {
                return Ok(None);
            }
            for (idx, arg) in candidate_args.into_iter().enumerate() {
                all_args[idx].push(arg);
            }
        }

        let mut merged_args = Vec::with_capacity(arity);
        for arg_candidates in all_args {
            let mut concrete_candidates = Vec::new();
            let mut unresolved_candidates = Vec::new();
            for arg in arg_candidates {
                if self.is_deferred_constructor_literal_payload_type(&arg) {
                    let literal_default = BuiltinTypes::integer();
                    if !concrete_candidates
                        .iter()
                        .any(|existing| self.types_equal(existing, &literal_default))
                    {
                        concrete_candidates.push(literal_default);
                    }
                    unresolved_candidates.push(arg);
                    continue;
                }
                if self.type_contains_unresolved_vars(&arg) {
                    unresolved_candidates.push(arg);
                    continue;
                }
                if !concrete_candidates
                    .iter()
                    .any(|existing| self.types_equal(existing, &arg))
                {
                    concrete_candidates.push(arg);
                }
            }

            if concrete_candidates.is_empty() {
                if allow_unresolved_generic_args {
                    let representative = match unresolved_candidates.first().cloned() {
                        Some(c) => c,
                        None => self.fresh_type_var(),
                    };
                    for unresolved in unresolved_candidates.iter().skip(1) {
                        self.constraints
                            .push((representative.clone(), unresolved.clone()));
                    }
                    merged_args.push(representative);
                    continue;
                }

                let base_name = match &base {
                    Type::Concrete(ann) if ann.as_type_name_str().is_some() => {
                        ann.as_type_name_str().unwrap().to_string()
                    }
                    _ => "generic".to_string(),
                };
                return Err(TypeError::GenericTypeError {
                    message: format!("Could not infer generic type arguments for '{}'", base_name),
                    symbol: Some(base_name),
                });
            }

            let merged_arg = if concrete_candidates.len() == 1 {
                concrete_candidates.into_iter().next().unwrap()
            } else {
                self.create_nominal_union(&concrete_candidates)?
            };
            for unresolved in unresolved_candidates {
                self.constraints.push((unresolved, merged_arg.clone()));
            }
            merged_args.push(merged_arg);
        }

        Ok(Some(Type::Generic {
            base: Box::new(base),
            args: merged_args,
        }))
    }

    /// Join branch/return values that are all callable signatures.
    ///
    /// This is deliberately narrower than `types_equal`: a value-level
    /// `if`/`match` that selects among closures is valid only when every arm is
    /// callable with the same arity and the ordinary solver can unify each
    /// parameter and return slot. The returned representative is the first
    /// callable signature; constraints connect every sibling arm to it, so
    /// `int` versus `number` still rejects through the normal unifier.
    pub(crate) fn try_join_callable_types(&mut self, types: &[Type]) -> Option<Type> {
        let first = types.first()?;
        let Type::Function {
            params: head_params,
            returns: head_returns,
        } = first
        else {
            return None;
        };

        let head_params = head_params.clone();
        let head_return = head_returns.as_ref().clone();

        for ty in types.iter().skip(1) {
            let Type::Function { params, returns } = ty else {
                return None;
            };
            if params.len() != head_params.len() {
                return None;
            }
            for (head, other) in head_params.iter().cloned().zip(params.iter().cloned()) {
                self.constraints.push((head, other));
            }
            self.constraints
                .push((head_return.clone(), returns.as_ref().clone()));
        }

        Some(Type::Function {
            params: head_params,
            returns: Box::new(head_return),
        })
    }

    fn combine_return_types_internal(
        &mut self,
        candidates: &[Type],
        allow_unresolved_generic_args: bool,
    ) -> TypeResult<Type> {
        let mut unique = Vec::new();
        for ty in candidates {
            if !unique.iter().any(|existing| self.types_equal(existing, ty)) {
                unique.push(ty.clone());
            }
        }

        if unique.is_empty() {
            return Ok(BuiltinTypes::void());
        }
        if unique.len() == 1 {
            let only = unique.into_iter().next().unwrap();
            if !allow_unresolved_generic_args {
                self.ensure_no_unresolved_generic_args(&only)?;
            }
            return Ok(only);
        }

        if let Some(merged_generic) =
            self.merge_homogeneous_generic_types(&unique, allow_unresolved_generic_args)?
        {
            if !allow_unresolved_generic_args {
                self.ensure_no_unresolved_generic_args(&merged_generic)?;
            }
            return Ok(merged_generic);
        }

        if let Some(callable) = self.try_join_callable_types(&unique) {
            return Ok(callable);
        }

        if allow_unresolved_generic_args
            && unique
                .iter()
                .any(|ty| self.type_contains_unresolved_vars(ty))
        {
            let representative = unique[0].clone();
            for other in unique.iter().skip(1) {
                self.constraints
                    .push((representative.clone(), other.clone()));
            }
            return Ok(representative);
        }

        let union = self.create_nominal_union(&unique)?;
        if !allow_unresolved_generic_args {
            self.ensure_no_unresolved_generic_args(&union)?;
        }
        Ok(union)
    }

    pub(crate) fn combine_return_types(&mut self, candidates: &[Type]) -> TypeResult<Type> {
        self.combine_return_types_internal(candidates, false)
    }

    pub(crate) fn combine_return_types_allow_unresolved(
        &mut self,
        candidates: &[Type],
    ) -> TypeResult<Type> {
        self.combine_return_types_internal(candidates, true)
    }

    fn is_deferred_constructor_literal_payload_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Variable(var) | Type::Constrained { var, .. } => {
                self.deferred_constructor_literal_payload_vars.contains(var)
            }
            _ => false,
        }
    }

    /// Infer types for a complete program
    /// Run the optimistic hoisting pre-pass
    ///
    /// This collects all property assignments (e.g., `a.b = 2`) and registers
    /// them as hoisted fields so they're available during the main type checking pass.
    ///
    /// Call this BEFORE `infer_program` or `infer_expr` for optimistic hoisting to work.
    pub fn run_hoisting_prepass(&mut self, program: &Program) {
        use hoisting::PropertyAssignmentCollector;

        // Clear any previous hoisted fields
        self.env.clear_hoisted_fields();

        // Collect all property assignments
        let assignments = PropertyAssignmentCollector::collect(program);

        // For each assignment, infer the value type and register it
        for assignment in &assignments {
            // Try to infer the type of the assigned value
            // We use a best-effort approach - if inference fails, we skip hoisting this field
            if let Ok(field_type) = self.infer_expr(&assignment.value_expr) {
                self.env.register_hoisted_field(
                    &assignment.variable,
                    &assignment.property,
                    field_type,
                );
            }
        }
    }

    /// Infer types for a complete program
    ///
    /// This runs the hoisting pre-pass automatically before main type inference.
    pub fn infer_program(&mut self, program: &Program) -> TypeResult<HashMap<String, Type>> {
        let (types, errors) = self.infer_program_best_effort(program);
        if let Some(err) = errors.into_iter().next() {
            Err(err)
        } else {
            Ok(types)
        }
    }

    /// DESIGN §3.3 RESULTS-IDENTICAL differential-test support: type-check a
    /// CONSUMER program against a module interface that is ALREADY registered in
    /// this engine (by a from-source compile — `infer_program_best_effort` —
    /// OR by a cache replay — `replay_resolved_interface`).
    ///
    /// This mirrors `infer_program_best_effort`'s predeclare→infer→solve→
    /// substitute sequence EXACTLY, but DELIBERATELY OMITS the leading
    /// registration-state resets (`struct_type_defs.clear()`,
    /// `callable_origins_by_name.clear()`, …) so the previously-registered module
    /// interface (structs, fns, enums, traits, impls, method table) SURVIVES and
    /// drives consumer checking. It performs only the per-run scratch resets that
    /// are local to a single inference pass (return/union/callsite scratch), then
    /// re-seeds the builtin callable defaults — identical to the production path —
    /// so the only difference between the two differential routes is HOW M's
    /// interface entered the engine, never the consumer-check itself.
    ///
    /// Differential-test support: the production checker always starts from a
    /// clean engine via `infer_program_best_effort`. This additive-check entry
    /// has NO production caller and exists solely to make the §3.3
    /// RESULTS-IDENTICAL binder mechanically verifiable from a downstream crate
    /// (`shape-vm`'s `bundle_compiler` differential test), where a `#[cfg(test)]`
    /// method on this crate would be invisible across the crate boundary. It is
    /// pure type-checking — no dynamic dispatch, no runtime carrier — so carries
    /// no forbidden-pattern surface.
    pub fn check_consumer_against_registered_interface(
        &mut self,
        consumer: &Program,
    ) -> (HashMap<String, Type>, Vec<TypeError>) {
        // Per-run scratch resets ONLY (local to a single inference pass). These
        // are the same fields `infer_program_best_effort` resets; what we do NOT
        // touch here is the registration state (`struct_type_defs`, the
        // callable/property/variable origin maps) that carries the registered M
        // interface across into the consumer check.
        self.pending_return_unions.clear();
        self.callsite_return_types.clear();
        self.callable_param_source_vars.clear();
        self.callable_return_from_fn_param.clear();
        self.callable_array_return_from_fn_param.clear();
        self.callable_param_defaults.clear();
        self.callable_numeric_param_indices.clear();
        self.deferred_closure_numeric_param_vars.clear();
        self.deferred_closure_numeric_param_body_hint.clear();
        self.deferred_closure_numeric_binding_hints.clear();
        self.escaping_closure_numeric_param_vars.clear();
        self.escaping_closure_arg_sites.clear();
        self.deferred_constructor_literal_payload_vars.clear();
        Self::seed_builtin_callable_defaults(&mut self.callable_param_defaults);
        self.return_var_aliases.clear();
        self.return_scopes.clear();
        self.implicit_return_scopes.clear();
        self.unannotated_let_binding_origins.clear();
        self.expr_type_table.clear();
        self.binding_fact_table.clear();
        self.param_destructure_array_element_links.clear();
        self.param_destructure_field_links.clear();

        // Hoisting pre-pass over the consumer (mirrors production).
        self.run_hoisting_prepass(consumer);

        let mut types = HashMap::new();
        let mut errors = Vec::new();

        for item in &consumer.items {
            if let Err(err) = self.predeclare_nominal_type_item(item) {
                errors.push(err);
            }
        }
        for item in &consumer.items {
            if let Err(err) = self.predeclare_item(item) {
                errors.push(err);
            }
        }
        // Declaration-order-independent operator-trait dispatch (mirrors the
        // production `infer_program_best_effort` path).
        self.register_traits_and_impls_prepass(&consumer.items);
        for item in &consumer.items {
            if let Err(err) = self.infer_item(item, &mut types) {
                errors.push(err);
            }
        }

        // Indirected-callable COMPLETENESS parity with `infer_program_best_effort`
        // (the production path). Follow each escaping closure-arg callable through
        // indirection so the differential consumer-check resolves the same way the
        // from-source compile does; an un-followable hop still SURFACEs below.
        self.record_transitive_hof_return_aliases(consumer);
        self.resolve_indirected_closure_arg_params(consumer);

        self.solver.set_method_table(self.method_table.clone());
        self.solver.set_trait_impls(self.env.trait_impl_keys());
        if let Err(err) = self.solver.solve(&mut self.constraints) {
            errors.push(err);
        }

        self.apply_callsite_unions(&mut types);
        errors.extend(self.refine_numeric_params_post_callsite(&mut types));
        // ROOT-2: closure params that no call site resolved fall back to `number`
        // (never-invoked closure) or SURFACE (indirected-callable: escaped into a
        // user call but never pinned — see `escaping_closure_numeric_param_vars`).
        errors.extend(self.default_unresolved_closure_numeric_params());
        self.default_unresolved_constructor_literal_payload_vars();
        errors.extend(self.propagate_param_destructure_field_links());

        for (_name, ty) in types.iter_mut() {
            *ty = self.solver.unifier().apply_substitutions(ty);
        }

        // Let-gen spec §4 (A-enforced): parity with `infer_program_best_effort`.
        errors.extend(self.reject_unpinnable_let_bindings(&types));

        // T1 keystone: rewrite the per-expression type table through the final
        // substitution and drop un-inferable entries (parity with production).
        self.finalize_expr_type_table();
        self.finalize_binding_fact_table();

        (types, errors)
    }

    /// Infer types for a complete program and keep successful inferences even
    /// when some items fail type checking.
    ///
    /// This uses the same inference engine and constraint solver as `infer_program`
    /// and is intended for tooling surfaces that should avoid guessing.
    pub fn infer_program_best_effort(
        &mut self,
        program: &Program,
    ) -> (HashMap<String, Type>, Vec<TypeError>) {
        self.pending_return_unions.clear();
        self.callsite_return_types.clear();
        self.callable_param_source_vars.clear();
        self.callable_return_from_fn_param.clear();
        self.callable_array_return_from_fn_param.clear();
        self.callable_param_defaults.clear();
        self.callable_numeric_param_indices.clear();
        self.deferred_closure_numeric_param_vars.clear();
        self.deferred_closure_numeric_param_body_hint.clear();
        self.deferred_closure_numeric_binding_hints.clear();
        self.escaping_closure_numeric_param_vars.clear();
        self.escaping_closure_arg_sites.clear();
        self.deferred_constructor_literal_payload_vars.clear();
        Self::seed_builtin_callable_defaults(&mut self.callable_param_defaults);
        self.return_var_aliases.clear();
        self.return_scopes.clear();
        self.implicit_return_scopes.clear();
        self.struct_type_defs.clear();
        self.constraint_origins.clear();
        self.callable_origins_by_name.clear();
        self.unknown_property_origins.clear();
        self.undefined_variable_origins.clear();
        self.non_exhaustive_match_origins.clear();
        self.unannotated_let_binding_origins.clear();
        self.expr_type_table.clear();
        self.binding_fact_table.clear();
        self.param_destructure_array_element_links.clear();
        self.param_destructure_field_links.clear();
        // Run hoisting pre-pass first
        self.run_hoisting_prepass(program);

        let mut types = HashMap::new();
        let mut errors = Vec::new();

        // First pass: predeclare nominal type definitions, then callable
        // symbols/methods so references are order-independent (matches compiler
        // front-end behavior).
        for item in &program.items {
            if let Err(err) = self.predeclare_nominal_type_item(item) {
                errors.push(err);
            }
        }
        for item in &program.items {
            if let Err(err) = self.predeclare_item(item) {
                errors.push(err);
            }
        }

        // Register every trait + impl + enum across the WHOLE program before
        // function bodies are inferred, so operator-trait dispatch is
        // declaration-ORDER INDEPENDENT (a `fn` that uses `impl Add for T`
        // resolves whether the impl is textually before or after it). Idempotent
        // with the per-item registration in `infer_item`, which stays the
        // canonical (source-order) error site.
        self.register_traits_and_impls_prepass(&program.items);

        for item in &program.items {
            if let Err(err) = self.infer_item(item, &mut types) {
                errors.push(err);
            }
        }

        // HOF-return aliasing, TRANSITIVELY (the indirected-callable root).
        //
        // `infer_item` records `callable_return_from_fn_param[F] = j` only when
        // F's tail value DIRECTLY invokes its own fn-typed param j
        // (`fn applyx(f,x,y){ f(x,y) }`). A wrapper that forwards through ANOTHER
        // named HOF (`fn wrap(f,x,y){ applyx(f,x,y) }`) is NOT caught: its tail
        // calls `applyx` (a named fn, not a param), so `wrap`'s return var is
        // left FREE. Closing the indirection here lets the existing post-solve
        // hof-return re-solve (below) pin `wrap`'s return to the SAME genuine
        // fn-typed-param return its callee already aliases to — so a forwarded
        // wrapper whose callable DOES resolve infers correctly instead of being
        // left an `unknown` the emitter lowers to a generic `CallMethod("add")`.
        // (The genuinely-un-inferable case — a closure that escaped indirectly
        // and never pinned — is rejected at its source by
        // `default_unresolved_closure_numeric_params` via the
        // `escaping_closure_numeric_param_vars` discriminator, not here.)
        //
        // Pure AST + already-recorded map; fixpoint bounded by the chain depth.
        self.record_transitive_hof_return_aliases(program);

        // Indirected-callable COMPLETENESS pass (full-inference ruling). FOLLOW
        // each escaping closure-arg callable through indirection (forwarding
        // wrapper / id-laundered let) to its concrete invocation and push
        // `closure_param_var ~ <proven arg type>` constraints, so the closure's
        // numeric params resolve in the solve below instead of reaching the
        // `default_unresolved_closure_numeric_params` SURFACE. A hop the pass
        // cannot follow pushes nothing — the SoundRoot floor still rejects it.
        self.resolve_indirected_closure_arg_params(program);

        // Attach the method table and trait impl data to the solver,
        // then solve all constraints
        self.solver.set_method_table(self.method_table.clone());
        self.solver.set_trait_impls(self.env.trait_impl_keys());
        // Named-struct field schemas let the solver unify a nominal struct
        // type (`Point`) with the structural object type its instances carry
        // (`{ x: number, y: number }`). A declared `fn f(p: Point)` param
        // resolves through the struct's type alias to `Object([x, y])`, while
        // a `Point { .. }` literal stays nominal as `Reference("Point")`; the
        // two must unify at the call site. Comptime fields are excluded —
        // they occupy zero runtime slots, matching the alias construction in
        // `infer_item`/`predeclare_struct_type`.
        let struct_schemas: HashMap<String, Vec<ObjectTypeField>> = self
            .struct_type_defs
            .iter()
            .map(|(name, def)| {
                let fields = def
                    .fields
                    .iter()
                    .filter(|f| !f.is_comptime)
                    .map(|f| ObjectTypeField {
                        name: f.name.clone(),
                        optional: f.default_value.is_some(),
                        type_annotation: f.type_annotation.clone(),
                        annotations: vec![],
                    })
                    .collect();
                (name.clone(), fields)
            })
            .collect();
        self.solver.set_struct_schemas(struct_schemas);
        if let Err(err) = self.solver.solve(&mut self.constraints) {
            errors.push(err);
        }

        // Apply callsite widening before root-scope substitutions so unresolved
        // callable vars can still be widened in best-effort mode.
        self.apply_callsite_unions(&mut types);

        // Last-resort numeric default: any `Numeric`-bounded parameter that
        // transitive callsite propagation still could not resolve collapses
        // to `number` (the deferred half of the split in `infer_function`).
        // This step also enforces the `Numeric` bound itself: a parameter
        // whose body imposes `Numeric` must not be widened to a non-numeric
        // type by callsite propagation — that mismatch is a type error.
        errors.extend(self.refine_numeric_params_post_callsite(&mut types));
        // ROOT-2: closure params that no call site resolved fall back to `number`
        // (never-invoked closure) or SURFACE (indirected-callable: escaped into a
        // user call but never pinned — see `escaping_closure_numeric_param_vars`).
        errors.extend(self.default_unresolved_closure_numeric_params());
        self.default_unresolved_constructor_literal_payload_vars();

        // v0.3.3 ref-param caller->param inference (second ACU pass). The
        // first `apply_callsite_unions` runs before `refine_numeric_*` and the
        // closure/constructor defaults; an observed call-site argument whose
        // type variable only becomes concrete THROUGH one of those later steps
        // (e.g. a `let mut i = 1` loop counter whose var chases to `int` only
        // once the surrounding numeric chain is fully pinned) was therefore
        // still a bare variable at the first union and produced an empty union.
        // A SECOND pass after every binding-adding step has been applied lets
        // such a now-concrete observed argument resolve the corresponding
        // by-value parameter — the `val` of `fn add_to(&sum, val) { sum = sum +
        // val }` called as `add_to(&total, i)` in a `while` loop. (The for-in
        // sibling already passed because the loop element type is concrete at
        // the call site; only the `let mut` counter chained through the late
        // numeric default.) This is the same union mechanism, re-applied once
        // more — no new opcode, no fabricated kind, no value widening; a
        // CONFLICTING observed pair still produces the genuine union mismatch.
        self.apply_callsite_unions(&mut types);
        errors.extend(self.propagate_param_destructure_field_links());
        self.rewalk_resolved_function_bodies(program, &mut types);

        // HOF return-type soundness re-check (the sg2 root, int/number guard).
        //
        // `apply_callsite_unions` resolved each HOF wrapper's return type to its
        // fn-typed param's GENUINE return type (`apply2` returns `f`'s return —
        // `int` for `apply2(|a,b| a*b, …)`). But that resolution is post-solve:
        // a USE site (`let n: number = 1.0; n + apply2(|a,b| a*b, 6, 7)`)
        // already unified the wrapper's still-free return var against its
        // own demanded type (`number`) during `solver.solve`, so the genuine
        // `int` was never checked against it. Without this guard the bytecode
        // emitter would then see `number + int` and widen via `IntToNumber` —
        // exactly the deleted implicit int->number coercion (CLAUDE.md: int and
        // number do NOT unify).
        //
        // Re-pushing the genuine `return_var ~ R` constraints and re-solving
        // makes such a conflict a real type error: the solver already bound the
        // return var to `number` at the use site, so `number ~ int` rejects.
        // When the use site agrees (`acc: int; acc + apply2(…)`) the constraint
        // is a no-op. When no use site pinned the var, it simply binds to the
        // genuine `R` (int stays int, number stays number — no defaulting).
        let mut hof_return_constraints: Vec<(Type, Type)> = Vec::new();
        for (fn_name, &fn_param_idx) in &self.callable_return_from_fn_param {
            let Some(Type::Function { params, returns }) = types.get(fn_name) else {
                continue;
            };
            let Type::Variable(_) = returns.as_ref() else {
                // Already concrete in `types` — the genuine return is what the
                // emitter will read; nothing to re-assert.
                continue;
            };
            let Some(Type::Function {
                returns: param_returns,
                ..
            }) = params.get(fn_param_idx)
            else {
                continue;
            };
            let genuine = self
                .solver
                .unifier()
                .apply_substitutions(param_returns.as_ref());
            if matches!(genuine, Type::Variable(_)) {
                continue;
            }
            hof_return_constraints.push(((**returns).clone(), genuine));
        }
        for (fn_name, &fn_param_idx) in &self.callable_array_return_from_fn_param {
            let Some(Type::Function { params, returns }) = types.get(fn_name) else {
                continue;
            };
            let Some(Type::Function {
                returns: param_returns,
                ..
            }) = params.get(fn_param_idx)
            else {
                continue;
            };
            let genuine = self
                .solver
                .unifier()
                .apply_substitutions(param_returns.as_ref());
            if matches!(genuine, Type::Variable(_) | Type::Constrained { .. }) {
                continue;
            }
            hof_return_constraints.push(((**returns).clone(), BuiltinTypes::array(genuine)));
        }
        if !hof_return_constraints.is_empty() {
            if let Err(err) = self.solver.solve(&mut hof_return_constraints) {
                errors.push(err);
            }
        }
        errors.extend(self.propagate_resolved_call_returns_to_callsite_instances(&types));

        // Apply substitutions to get final types
        for (_name, ty) in types.iter_mut() {
            *ty = self.solver.unifier().apply_substitutions(ty);
        }

        // Let-gen spec §4 (A-enforced): now that every binding has its FINAL
        // type, reject any module-scope un-annotated `let` whose type still
        // carries an un-pinnable generic argument.
        errors.extend(self.reject_unpinnable_let_bindings(&types));

        // T1 keystone (strict-flip, 2026-06-22): now that the unifier carries
        // every solver binding, rewrite the per-expression type table through
        // the final substitution and drop entries that remain un-inferable.
        self.finalize_expr_type_table();
        self.finalize_binding_fact_table();

        (types, errors)
    }

    /// Infer a program once and package the finalized handoff facts.
    pub fn infer_program_facts_best_effort(
        &mut self,
        program: &Program,
    ) -> (InferenceFacts, Vec<TypeError>) {
        let (types, errors) = self.infer_program_best_effort(program);
        let expression_types = self.take_expr_type_table();
        let binding_facts = self.take_binding_fact_table();
        (
            InferenceFacts::with_binding_facts(types, expression_types, binding_facts),
            errors,
        )
    }

    /// Extend `callable_return_from_fn_param` across one-named-function-hop
    /// forwarding so an indirected HOF wrapper's return is aliased to the SAME
    /// fn-typed-param return its callee already aliases to (the
    /// `fn wrap(f,x,y){ applyx(f,x,y) }` → `applyx`'s param-`f` return chain).
    ///
    /// `infer_item` records only the DIRECT shape (tail invokes the function's
    /// own param). This fixpoint adds F→k whenever F's tail is `g(a0,a1,…)`, g
    /// is already recorded with index j, and `a_j` is F's own unannotated param
    /// at index k. Pure AST; conservative (same gates as the direct recorder:
    /// unannotated fn, no explicit `return`, single tail value that is a direct
    /// `g(...)` call); bounded by the forwarding-chain depth.
    fn record_transitive_hof_return_aliases(&mut self, program: &Program) {
        use shape_ast::ast::{Expr, Item};

        // Index every named function declaration for tail-shape inspection.
        let funcs: Vec<&shape_ast::ast::FunctionDef> = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Function(func, _) => Some(func),
                _ => None,
            })
            .collect();

        const MAX_ROUNDS: usize = 16;
        let max_rounds = funcs.len().saturating_add(1).min(MAX_ROUNDS);
        for _round in 0..max_rounds {
            let mut changed = false;
            for func in &funcs {
                if self.callable_return_from_fn_param.contains_key(&func.name) {
                    continue;
                }
                if func.return_type.is_some() {
                    continue;
                }
                // Map this function's UNANNOTATED single-identifier params to
                // their index (the candidate forwarded callable positions).
                let mut unannotated_param_index: HashMap<String, usize> = HashMap::new();
                for (k, p) in func.params.iter().enumerate() {
                    if p.type_annotation.is_some() {
                        continue;
                    }
                    let names = p.get_identifiers();
                    if names.len() == 1 {
                        unannotated_param_index.insert(names[0].clone(), k);
                    }
                }
                if unannotated_param_index.is_empty() {
                    continue;
                }
                // Same direct-shape gate as the recorder: no explicit returns,
                // exactly one tail value, and it is a direct named call.
                let mut explicit_returns: Vec<&Expr> = Vec::new();
                Self::collect_explicit_returns(&func.body, &mut explicit_returns);
                if !explicit_returns.is_empty() {
                    continue;
                }
                let mut tail_values: Vec<&Expr> = Vec::new();
                Self::collect_tail_values(&func.body, &mut tail_values);
                if tail_values.len() != 1 {
                    continue;
                }
                let Expr::FunctionCall {
                    name: callee_name,
                    args,
                    ..
                } = tail_values[0]
                else {
                    continue;
                };
                // The callee must itself alias its return to its fn-typed param j.
                let Some(&callee_param_idx) =
                    self.callable_return_from_fn_param.get(callee_name.as_str())
                else {
                    continue;
                };
                // The argument forwarded at that position must be one of THIS
                // function's own unannotated params (a bare identifier).
                let Some(Expr::Identifier(arg_name, _)) = args.get(callee_param_idx) else {
                    continue;
                };
                if let Some(&k) = unannotated_param_index.get(arg_name.as_str()) {
                    self.callable_return_from_fn_param
                        .insert(func.name.clone(), k);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Indirected-callable COMPLETENESS pass (full-inference ruling). For each
    /// recorded `escaping_closure_arg_sites` entry, FOLLOW the callable through
    /// indirection to a concrete invocation whose argument types prove the
    /// closure's param types, and push `closure_param_var[k] ~ <proven type>`.
    ///
    /// Two indirection shapes are followed, both purely from the program AST +
    /// the already-recorded `callable_return_from_fn_param` map:
    ///
    ///  (1) FORWARDING WRAPPER. The closure is passed to `outer` at position p;
    ///      `outer`'s body invokes its own param p as a callable — directly
    ///      (`fn applyx(f,x,y){ f(x,y) }`) or by forwarding it one hop into
    ///      another named callee that itself invokes its param p
    ///      (`fn wrap(f,x,y){ applyx(f,x,y) }`). The invocation's argument slots
    ///      map back to `outer`'s OWN params, whose types are this very call
    ///      site's sibling arguments (`wrap(|a,b| a*b, 6, 7)` → 6,7 = int).
    ///
    ///  (2) IDENTITY LAUNDER. The closure is passed to an identity-like `id`
    ///      (`fn id(g){ g }`, recorded `callable_return_from_fn_param[id]=p`)
    ///      whose result is bound to a `let` and later USED as the callable
    ///      argument of a wrapper that DOES invoke it
    ///      (`let h = id(|a,b| a*b); applyx(h, 6, 7)`). The invocation argument
    ///      types come from that downstream call site (6,7 = int).
    ///
    /// In both shapes the proven argument types are COPIED onto the closure's
    /// param vars (int stays int, number stays number — never a `number`
    /// default). A site whose callable the pass cannot follow to a concrete
    /// invocation is left untouched, so `default_unresolved_closure_numeric_params`
    /// still SURFACEs it — the SoundRoot floor is preserved.
    fn resolve_indirected_closure_arg_params(&mut self, program: &Program) {
        use shape_ast::ast::Item;

        let sites = std::mem::take(&mut self.escaping_closure_arg_sites);
        if sites.is_empty() {
            return;
        }

        let funcs: HashMap<String, &shape_ast::ast::FunctionDef> = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Function(func, _) => Some((func.name.clone(), func)),
                _ => None,
            })
            .collect();

        let mut to_push: Vec<(Type, Type)> = Vec::new();

        for (callee_name, closure_arg_idx, closure_param_vars, call_args) in &sites {
            // (1) FORWARDING WRAPPER: the callee invokes its own param
            // `closure_arg_idx` (directly or via one forwarding hop) with a known
            // outer-param-index mapping. Map those outer params back to THIS
            // call's sibling argument types.
            if let Some(outer_indices) =
                self.callable_invocation_outer_arg_indices(&funcs, callee_name, *closure_arg_idx)
            {
                if outer_indices.len() == closure_param_vars.len() {
                    if let Some(arg_types) =
                        self.concrete_arg_types_at_indices(call_args, &outer_indices)
                    {
                        for (var, ty) in closure_param_vars.iter().zip(arg_types.into_iter()) {
                            to_push.push((Type::Variable(var.clone()), ty));
                        }
                        continue;
                    }
                }
            }

            // (2) IDENTITY LAUNDER: the callee just returns its param at
            // `closure_arg_idx` (`fn id(g){ g }` — tail is the bare param). Find
            // the downstream USE of the laundered binding as a callable argument
            // of a wrapper that DOES invoke it, and read the invocation argument
            // types there.
            if Self::fn_returns_param_directly(&funcs, callee_name, *closure_arg_idx) {
                if let Some(arg_types) =
                    self.laundered_closure_invocation_arg_types(program, &funcs, callee_name)
                {
                    if arg_types.len() == closure_param_vars.len() {
                        for (var, ty) in closure_param_vars.iter().zip(arg_types.into_iter()) {
                            to_push.push((Type::Variable(var.clone()), ty));
                        }
                        continue;
                    }
                }
            }

            // (3) RETURNED CLOSURE WRAPPER: the callee returns a closure whose
            // body invokes its callable param (`fn flip(f){ |a,b| f(b,a) }`).
            // A later call of the returned binding pins the returned closure's
            // params; map those positions back onto the original closure arg.
            if let Some(returned_param_indices) = self.returned_closure_callable_arg_param_indices(
                &funcs,
                callee_name,
                *closure_arg_idx,
            ) {
                if returned_param_indices.len() == closure_param_vars.len() {
                    if let Some(returned_arg_types) =
                        self.returned_closure_invocation_arg_types(program, callee_name)
                    {
                        let mut mapped = Vec::with_capacity(closure_param_vars.len());
                        let mut ok = true;
                        for &idx in &returned_param_indices {
                            let Some(ty) = returned_arg_types.get(idx).cloned() else {
                                ok = false;
                                break;
                            };
                            mapped.push(ty);
                        }
                        if ok {
                            for (var, ty) in closure_param_vars.iter().zip(mapped.into_iter()) {
                                to_push.push((Type::Variable(var.clone()), ty));
                            }
                            continue;
                        }
                    }
                }
            }
        }

        for (lhs, rhs) in to_push {
            self.constraints.push((lhs, rhs));
        }
    }

    /// Return the OUTER-param indices of `callee_name` that its callable param
    /// `callable_param_idx` is invoked with — following at most one named
    /// forwarding hop. `None` when no sound mapping exists (the callable is not
    /// invoked, or is invoked with non-trivial args).
    ///
    /// DIRECT (`fn applyx(f,x,y){ f(x,y) }`, callable param 0): the body call
    /// `f(x,y)` maps to outer params `[1,2]`.
    ///
    /// FORWARDING (`fn wrap(f,x,y){ applyx(f,x,y) }`, callable param 0): the body
    /// forwards `f` into `applyx` at position 0; `applyx` invokes ITS param 0
    /// with applyx-outer-indices `[1,2]`. Those map through `wrap`'s forwarding
    /// call `applyx(f,x,y)` — applyx arg 1 is `wrap`'s `x` (index 1), applyx arg
    /// 2 is `wrap`'s `y` (index 2) — back to `wrap`'s outer params `[1,2]`.
    fn callable_invocation_outer_arg_indices(
        &self,
        funcs: &HashMap<String, &shape_ast::ast::FunctionDef>,
        callee_name: &str,
        callable_param_idx: usize,
    ) -> Option<Vec<usize>> {
        let func = funcs.get(callee_name)?;
        let callable_name = func.params.get(callable_param_idx)?.simple_name()?;
        if func
            .params
            .get(callable_param_idx)?
            .type_annotation
            .is_some()
        {
            return None;
        }

        // Map this function's param names to their indices.
        let mut name_to_index: HashMap<&str, usize> = HashMap::new();
        for (i, p) in func.params.iter().enumerate() {
            if let Some(n) = p.simple_name() {
                name_to_index.insert(n, i);
            }
        }

        let shape_ast::ast::Expr::FunctionCall {
            name: tail_callee,
            args: tail_args,
            ..
        } = Self::single_return_or_tail_expr(&func.body)?
        else {
            return None;
        };

        // DIRECT: the tail call IS the callable itself (`f(x, y)`).
        if tail_callee == callable_name {
            return Self::map_call_args_to_outer_indices(tail_args, &name_to_index);
        }

        // FORWARDING (one hop): the tail call is some OTHER named function that
        // receives `callable_name` at one position and invokes ITS param there.
        let forwarded_pos = tail_args.iter().position(
            |a| matches!(a, shape_ast::ast::Expr::Identifier(id, _) if id == callable_name),
        )?;
        let inner_indices =
            self.callable_invocation_outer_arg_indices(funcs, tail_callee, forwarded_pos)?;
        // `inner_indices` are indices into the INNER callee's params. Map each
        // back through this forwarding call's args to THIS function's params.
        let mut mapped = Vec::with_capacity(inner_indices.len());
        for &inner_idx in &inner_indices {
            let shape_ast::ast::Expr::Identifier(arg_name, _) = tail_args.get(inner_idx)? else {
                return None;
            };
            mapped.push(*name_to_index.get(arg_name.as_str())?);
        }
        Some(mapped)
    }

    fn single_return_or_tail_expr(
        body: &[shape_ast::ast::Statement],
    ) -> Option<&shape_ast::ast::Expr> {
        let mut explicit: Vec<&shape_ast::ast::Expr> = Vec::new();
        Self::collect_explicit_returns(body, &mut explicit);
        match explicit.as_slice() {
            [expr] => return Some(*expr),
            [] => {}
            _ => return None,
        }

        let mut tail_values: Vec<&shape_ast::ast::Expr> = Vec::new();
        Self::collect_tail_values(body, &mut tail_values);
        match tail_values.as_slice() {
            [expr] => Some(*expr),
            _ => None,
        }
    }

    fn returned_closure_callable_arg_param_indices(
        &self,
        funcs: &HashMap<String, &shape_ast::ast::FunctionDef>,
        callee_name: &str,
        callable_param_idx: usize,
    ) -> Option<Vec<usize>> {
        let func = funcs.get(callee_name)?;
        let callable_name = func.params.get(callable_param_idx)?.simple_name()?;
        if func
            .params
            .get(callable_param_idx)?
            .type_annotation
            .is_some()
        {
            return None;
        }

        let shape_ast::ast::Expr::FunctionExpr { params, body, .. } =
            Self::single_return_or_tail_expr(&func.body)?
        else {
            return None;
        };

        let mut closure_param_index: HashMap<&str, usize> = HashMap::new();
        for (idx, param) in params.iter().enumerate() {
            let name = param.simple_name()?;
            closure_param_index.insert(name, idx);
        }

        let shape_ast::ast::Expr::FunctionCall { name, args, .. } =
            Self::single_return_or_tail_expr(body)?
        else {
            return None;
        };
        if name != callable_name {
            return None;
        }

        let mut out = Vec::with_capacity(args.len());
        for arg in args {
            let shape_ast::ast::Expr::Identifier(id, _) = arg else {
                return None;
            };
            out.push(*closure_param_index.get(id.as_str())?);
        }
        Some(out)
    }

    /// True when `func_name`'s body is identity-like for its param at
    /// `param_idx`: an unannotated function with no explicit returns whose single
    /// tail value is the bare param identifier (`fn id(g){ g }`). This is the
    /// id-launder shape — the callable passes through unchanged.
    fn fn_returns_param_directly(
        funcs: &HashMap<String, &shape_ast::ast::FunctionDef>,
        func_name: &str,
        param_idx: usize,
    ) -> bool {
        let Some(func) = funcs.get(func_name) else {
            return false;
        };
        if func.return_type.is_some() {
            return false;
        }
        let Some(param_name) = func.params.get(param_idx).and_then(|p| p.simple_name()) else {
            return false;
        };
        if func.params[param_idx].type_annotation.is_some() {
            return false;
        }
        matches!(
            Self::single_return_or_tail_expr(&func.body),
            Some(shape_ast::ast::Expr::Identifier(id, _)) if id == param_name
        )
    }

    /// Map a body-call's arguments to outer-param indices: every arg must be a
    /// bare identifier naming one of this function's params. `None` otherwise.
    fn map_call_args_to_outer_indices(
        call_args: &[shape_ast::ast::Expr],
        name_to_index: &HashMap<&str, usize>,
    ) -> Option<Vec<usize>> {
        let mut out = Vec::with_capacity(call_args.len());
        for a in call_args {
            let shape_ast::ast::Expr::Identifier(id, _) = a else {
                return None;
            };
            out.push(*name_to_index.get(id.as_str())?);
        }
        Some(out)
    }

    /// Read the already-inferred types of `call_args` at the given positions,
    /// requiring every one to resolve to a CONCRETE type (not a variable). This
    /// is the proven sibling-argument type the closure param adopts. `None` if
    /// any position is missing or still a variable — that hop is intractable and
    /// must be left to SURFACE.
    fn concrete_arg_types_at_indices(
        &mut self,
        call_args: &[shape_ast::ast::Expr],
        indices: &[usize],
    ) -> Option<Vec<Type>> {
        let mut out = Vec::with_capacity(indices.len());
        for &idx in indices {
            let arg = call_args.get(idx)?;
            let ty = self.infer_expr(arg).ok()?;
            let resolved = self.solver.unifier().apply_substitutions(&ty);
            if matches!(resolved, Type::Variable(_) | Type::Constrained { .. }) {
                return None;
            }
            out.push(resolved);
        }
        Some(out)
    }

    fn returned_closure_invocation_arg_types(
        &mut self,
        program: &Program,
        returned_from_name: &str,
    ) -> Option<Vec<Type>> {
        use shape_ast::ast::{Expr, Item, Statement};

        let mut result: Option<Vec<Type>> = None;

        fn scan_stmts(
            engine: &mut TypeInferenceEngine,
            returned_from_name: &str,
            stmts: &[Statement],
            result: &mut Option<Vec<Type>>,
        ) {
            let mut returned_bindings: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            for stmt in stmts {
                if result.is_some() {
                    return;
                }
                match stmt {
                    Statement::VariableDecl(decl, _) => {
                        if let Some(Expr::FunctionCall { name, .. }) = decl.value.as_ref() {
                            if name == returned_from_name {
                                for binding in decl.pattern.get_identifiers() {
                                    returned_bindings.insert(binding);
                                }
                                continue;
                            }
                        }
                        if let Some(value) = decl.value.as_ref() {
                            try_use(engine, &returned_bindings, value, result);
                        }
                    }
                    Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                        try_use(engine, &returned_bindings, expr, result);
                    }
                    _ => {}
                }
            }
        }

        fn try_use(
            engine: &mut TypeInferenceEngine,
            returned_bindings: &std::collections::HashSet<String>,
            expr: &Expr,
            result: &mut Option<Vec<Type>>,
        ) {
            if result.is_some() {
                return;
            }
            let mut collector = CallCollector { calls: Vec::new() };
            crate::visitor::walk_expr(&mut collector, expr);
            for (name, args) in collector.calls {
                if !returned_bindings.contains(&name) {
                    continue;
                }
                let indices: Vec<usize> = (0..args.len()).collect();
                if let Some(types) = engine.concrete_arg_types_at_indices(&args, &indices) {
                    *result = Some(types);
                    return;
                }
            }
        }

        struct CallCollector {
            calls: Vec<(String, Vec<Expr>)>,
        }
        impl crate::visitor::Visitor for CallCollector {
            fn visit_expr(&mut self, expr: &Expr) -> bool {
                if let Expr::FunctionCall { name, args, .. } = expr {
                    self.calls.push((name.clone(), args.clone()));
                }
                true
            }
        }

        let mut top_level: Vec<Statement> = Vec::new();
        for item in &program.items {
            match item {
                Item::Statement(stmt, _) => top_level.push(stmt.clone()),
                Item::VariableDecl(decl, span) => {
                    top_level.push(Statement::VariableDecl(decl.clone(), *span))
                }
                Item::Expression(expr, span) => {
                    top_level.push(Statement::Expression(expr.clone(), *span))
                }
                Item::Assignment(assign, span) => {
                    top_level.push(Statement::Assignment(assign.clone(), *span))
                }
                _ => {}
            }
        }
        scan_stmts(self, returned_from_name, &top_level, &mut result);

        for item in &program.items {
            if result.is_some() {
                break;
            }
            if let Item::Function(func, _) = item {
                scan_stmts(self, returned_from_name, &func.body, &mut result);
            }
        }

        result
    }

    /// Identity-launder strategy. `id_name` is an identity-like function (returns
    /// its own param). Find a binding `let X = id_name(...)` whose result is later
    /// USED as the callable argument of a wrapper that invokes it, and return the
    /// concrete invocation argument types from that downstream call site.
    fn laundered_closure_invocation_arg_types(
        &mut self,
        program: &Program,
        funcs: &HashMap<String, &shape_ast::ast::FunctionDef>,
        id_name: &str,
    ) -> Option<Vec<Type>> {
        use shape_ast::ast::{Expr, Item, Statement};

        // Collect the candidate laundered binding names and their downstream
        // uses by walking every function body + the module top level.
        // A binding `let X = id_name(<closure-or-anything>)` makes `X` a
        // laundered callable; a later call `wrap(X, a, b)` where `wrap` invokes
        // its param at X's position pins the closure.
        let mut result: Option<Vec<Type>> = None;

        // Recursive statement walker collecting (binding_name -> laundered) and
        // resolving uses in the SAME statement list (lexical, forward-only).
        // We keep it simple: a single linear scan per body, tracking laundered
        // names seen so far.
        fn scan_stmts<'a>(
            engine: &mut TypeInferenceEngine,
            funcs: &HashMap<String, &shape_ast::ast::FunctionDef>,
            id_name: &str,
            stmts: &'a [Statement],
            result: &mut Option<Vec<Type>>,
        ) {
            let mut laundered: std::collections::HashSet<String> = std::collections::HashSet::new();
            for stmt in stmts {
                if result.is_some() {
                    return;
                }
                match stmt {
                    Statement::VariableDecl(decl, _) => {
                        if let Some(Expr::FunctionCall { name, .. }) = decl.value.as_ref() {
                            if name == id_name {
                                for n in decl.pattern.get_identifiers() {
                                    laundered.insert(n);
                                }
                                continue;
                            }
                        }
                        if let Some(v) = decl.value.as_ref() {
                            try_use(engine, funcs, &laundered, v, result);
                        }
                    }
                    Statement::Expression(e, _) => try_use(engine, funcs, &laundered, e, result),
                    Statement::Return(Some(e), _) => try_use(engine, funcs, &laundered, e, result),
                    _ => {}
                }
            }
        }

        // Inspect one expression for a wrapper call `wrap(<laundered>, …)` whose
        // param at the laundered position is invoked; read invocation arg types.
        fn try_use(
            engine: &mut TypeInferenceEngine,
            funcs: &HashMap<String, &shape_ast::ast::FunctionDef>,
            laundered: &std::collections::HashSet<String>,
            expr: &Expr,
            result: &mut Option<Vec<Type>>,
        ) {
            if result.is_some() {
                return;
            }
            // Collect every FunctionCall in the expr tree (the laundered use may
            // be nested inside a binary op / let initializer / block, e.g.
            // `let r = acc + applyx(h, 6, 7)`). Cloning the call shapes keeps the
            // immutable AST borrow separate from the `&mut engine` resolution.
            let mut collector = CallCollector { calls: Vec::new() };
            crate::visitor::walk_expr(&mut collector, expr);
            for (name, args) in collector.calls {
                for (pos, a) in args.iter().enumerate() {
                    if let Expr::Identifier(id, _) = a {
                        if laundered.contains(id) {
                            if let Some(outer_indices) =
                                engine.callable_invocation_outer_arg_indices(funcs, &name, pos)
                            {
                                if let Some(types) =
                                    engine.concrete_arg_types_at_indices(&args, &outer_indices)
                                {
                                    *result = Some(types);
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Pure-AST collector: gathers every `FunctionCall (name, args)` reachable
        // from an expression, so the laundered-binding use can be found wherever
        // it is nested. `visit_expr` returns `true` to keep descending.
        struct CallCollector {
            calls: Vec<(String, Vec<Expr>)>,
        }
        impl crate::visitor::Visitor for CallCollector {
            fn visit_expr(&mut self, expr: &Expr) -> bool {
                if let Expr::FunctionCall { name, args, .. } = expr {
                    self.calls.push((name.clone(), args.clone()));
                }
                true
            }
        }

        // Module top-level items form one flat statement sequence; lift each
        // into a `Statement` so the same lexical, forward-only `scan_stmts`
        // tracks laundered bindings declared at module scope, too.
        let mut top_level: Vec<Statement> = Vec::new();
        for item in &program.items {
            match item {
                Item::Statement(stmt, _) => top_level.push(stmt.clone()),
                Item::VariableDecl(decl, span) => {
                    top_level.push(Statement::VariableDecl(decl.clone(), *span))
                }
                Item::Expression(e, span) => {
                    top_level.push(Statement::Expression(e.clone(), *span))
                }
                Item::Assignment(a, span) => {
                    top_level.push(Statement::Assignment(a.clone(), *span))
                }
                _ => {}
            }
        }
        scan_stmts(self, funcs, id_name, &top_level, &mut result);

        for item in &program.items {
            if result.is_some() {
                break;
            }
            if let Item::Function(func, _) = item {
                scan_stmts(self, funcs, id_name, &func.body, &mut result);
            }
        }
        result
    }

    fn apply_callsite_unions(&mut self, types: &mut HashMap<String, Type>) {
        // Transitive callsite-union propagation.
        //
        // The base callsite-union scheme widens an unannotated parameter to the
        // union of *concrete* argument types observed at its call sites. That
        // cannot resolve a function reached only through nested/transitive
        // calls of other unannotated functions: every observed argument is
        // itself a still-unresolved `Type::Variable` (a caller's parameter
        // variable, or another callee's return variable), so the union is
        // empty and the parameter never widens.
        //
        // The fix is a fixpoint: a persistent `resolved` map records every
        // parameter-source variable (and, via the inferred return type, every
        // return variable) that has become concrete. `union_from_observed_types`
        // consults that map — through the unifier first, then `resolved`,
        // transitively — so an observed `Type::Variable` argument resolves once
        // some other function in the call graph has been concretized. Each
        // round can therefore unlock the next; iterating to a fixpoint
        // propagates concrete types along call edges in both directions
        // (`double`'s parameter ← `quad`'s parameter via the inner call;
        // `double`'s return → the outer call's argument).
        let callsites = self.callsite_param_types.clone();
        let mut resolved: HashMap<TypeVar, Type> = HashMap::new();

        // Bound the fixpoint. Each productive round either concretizes a
        // previously-unresolved function or records a new `resolved` entry;
        // the `changed` flag below breaks as soon as a round adds nothing, so
        // the common case is 1-2 rounds. The hard cap is a small constant —
        // it bounds the transitive call-graph depth the propagation can chase
        // (a 16-deep chain of unannotated functions is already pathological)
        // and guarantees termination without scaling with the (large, when
        // the stdlib is loaded) function count.
        const MAX_FIXPOINT_ROUNDS: usize = 16;
        let max_rounds = callsites.len().saturating_add(2).min(MAX_FIXPOINT_ROUNDS);
        for _round in 0..max_rounds {
            let mut changed = false;

            for (function_name, observed_by_param) in &callsites {
                let Some(Type::Function { params, returns }) = types.get(function_name) else {
                    continue;
                };
                // Clone out of `types` so the borrow ends before the
                // re-`insert` below.
                let mut widened_params = params.clone();
                let returns = (**returns).clone();
                let param_source_vars = self
                    .callable_param_source_vars
                    .get(function_name)
                    .cloned()
                    .unwrap_or_default();

                let mut substitutions: HashMap<TypeVar, Type> = HashMap::new();

                for (index, observed_types) in observed_by_param.iter().enumerate() {
                    if index >= widened_params.len() {
                        break;
                    }
                    let Some(widened_type) =
                        self.union_from_observed_types_with_resolved(observed_types, &resolved)
                    else {
                        continue;
                    };

                    let source_var = param_source_vars.get(index).and_then(|var| var.clone());
                    if let Some(var) = source_var.clone() {
                        substitutions.insert(var, widened_type.clone());
                    }

                    let current_param = widened_params[index].clone();
                    match current_param {
                        Type::Variable(var) => {
                            widened_params[index] = widened_type.clone();
                            substitutions.insert(var, widened_type);
                        }
                        _ if source_var.is_some() => {
                            widened_params[index] = widened_type;
                        }
                        _ => {}
                    }
                }

                // HOF return-type aliasing (the sg2 root). When this function's
                // return value is the result of invoking one of its own fn-typed
                // params in tail position (recorded in
                // `callable_return_from_fn_param` during body inference), and
                // that param has NOW widened to a concrete `Function { returns:
                // R }`, the function's still-unresolved return var resolves to
                // the EXACT `R`. This closes the post-solve ordering gap: the
                // body constraint linking the return var to the param's return
                // var was solved while the param was unresolved, so the return
                // var was never substituted. We adopt only a concrete `R` (int
                // stays int, number stays number); an unresolved `R` leaves the
                // return a variable, so the case SURFACEs unchanged.
                if let Type::Variable(return_var) = &returns {
                    if !substitutions.contains_key(return_var) {
                        if let Some(&fn_param_idx) =
                            self.callable_return_from_fn_param.get(function_name)
                        {
                            if let Some(Type::Function {
                                returns: param_returns,
                                ..
                            }) = widened_params.get(fn_param_idx)
                            {
                                let resolved_param_return = self
                                    .solver
                                    .unifier()
                                    .apply_substitutions(param_returns.as_ref());
                                if !matches!(resolved_param_return, Type::Variable(_)) {
                                    substitutions.insert(return_var.clone(), resolved_param_return);
                                }
                            }
                        }
                    }
                }

                self.propagate_return_alias_substitution(returns.clone(), &mut substitutions);
                let mut widened_return =
                    self.materialize_pending_return_union(returns.clone(), &substitutions);
                if let Some(&fn_param_idx) =
                    self.callable_array_return_from_fn_param.get(function_name)
                {
                    if let Some(Type::Function {
                        returns: param_returns,
                        ..
                    }) = widened_params.get(fn_param_idx)
                    {
                        let resolved_param_return = self
                            .solver
                            .unifier()
                            .apply_substitutions(param_returns.as_ref());
                        if !matches!(
                            resolved_param_return,
                            Type::Variable(_) | Type::Constrained { .. }
                        ) {
                            widened_return = BuiltinTypes::array(resolved_param_return);
                        }
                    }
                }

                let new_type = Type::Function {
                    params: widened_params.clone(),
                    returns: Box::new(widened_return.clone()),
                };
                if types.get(function_name) != Some(&new_type) {
                    changed = true;
                    types.insert(function_name.clone(), new_type);
                }

                // Record every parameter-source variable that became concrete
                // so transitively-called functions can resolve it next round.
                for (index, source_var) in param_source_vars.iter().enumerate() {
                    let Some(var) = source_var else { continue };
                    if let Some(param_ty) = widened_params.get(index) {
                        if !matches!(param_ty, Type::Variable(_)) {
                            match resolved.insert(var.clone(), param_ty.clone()) {
                                Some(prev) if &prev == param_ty => {}
                                _ => changed = true,
                            }
                        }
                    }
                }
                // Record the function's return variable → concrete return type
                // so a caller that passed this function's result as an argument
                // can resolve that argument next round.
                if let Type::Variable(return_var) = &returns {
                    if !matches!(widened_return, Type::Variable(_)) {
                        match resolved.insert(return_var.clone(), widened_return.clone()) {
                            Some(prev) if prev == widened_return => {}
                            _ => changed = true,
                        }
                    }
                }
            }

            if !changed {
                break;
            }
        }

        // Publish the fixpoint result into the unifier.
        //
        // `apply_callsite_unions` widens each function's `Type::Function`
        // entry in `types` directly, but the parameter-source variables it
        // resolved (`resolved`) were never visible to `self.unifier`. A
        // function that returns an object literal built from its parameters
        // — `fn aabb(lo, hi) { {min: lo, max: hi} }` — has those parameter
        // variables embedded as `tyvar` markers inside its return type's
        // `Object` annotation. The final `unifier.apply_substitutions` pass
        // (and every `infer_property_access` the bytecode compiler runs)
        // can only resolve those markers if the unifier knows the binding.
        //
        // Binding here closes that gap. An existing binding is respected:
        // when the unifier already resolved a variable to a *conflicting*
        // concrete type, that is a genuine type error surfaced elsewhere —
        // do not clobber it. A binding that merely refines an unresolved
        // variable, or agrees with the existing one, is published.
        //
        // HOF callee-param propagation (v0.3.3 c4-4D): the real fix lives
        // at the call site itself, in `propagate_hof_arg_callsites` in
        // `access.rs` — it records a synthetic callsite for a named
        // function passed as an HOF argument, derived from the body's
        // call-shape constraint on the outer callable. The component-wise
        // unification at this publish step is intentionally NOT applied:
        // an HM-generalized scheme is instantiated with fresh TypeVars at
        // each call site, so the widened-side variables here have no
        // back-link to the original (stored) source-vars of the inner
        // callee, and component-wise unification of the body-constraint
        // would route through fresh dead variables without helping. The
        // synthetic-callsite path goes through the existing widening +
        // numeric-refinement pipeline and writes the inner callee's stored
        // `Type::Function` entry in `types` directly.
        for (var, ty) in &resolved {
            if matches!(ty, Type::Variable(_)) {
                continue;
            }
            match self.solver.unifier().lookup(var) {
                Some(existing) if !matches!(existing, Type::Variable(_)) => {
                    // Already structurally bound — refine its inner variables
                    // with the callsite-proven type when possible. This is the
                    // parameter-destructure case: the function body first proves
                    // `p: Array<T_elem>` from `[a, b]`, then callsite union proves
                    // `p: Array<int>` from `sum_pair([10, 20])`; unifying those
                    // two static facts binds `T_elem = int` for binder facts.
                    let mut refinement = vec![(existing.clone(), ty.clone())];
                    let _ = self.solver.solve(&mut refinement);
                }
                _ => self.solver.unifier_mut().bind(var.clone(), ty.clone()),
            }
        }
    }

    /// `true` when `ty` satisfies the `Numeric` bound — a concrete numeric
    /// basic/reference type, or a union whose every member is numeric.
    fn type_satisfies_numeric_bound(ty: &Type) -> bool {
        let name_is_numeric = |ann: &TypeAnnotation| {
            ann.as_type_name_str()
                .is_some_and(BuiltinTypes::is_numeric_type_name)
        };
        match ty {
            Type::Concrete(ann @ (TypeAnnotation::Basic(_) | TypeAnnotation::Reference(_))) => {
                name_is_numeric(ann)
            }
            Type::Concrete(TypeAnnotation::Union(members)) => members.iter().all(name_is_numeric),
            _ => false,
        }
    }

    /// Last-resort `number` default for `Numeric`-bounded parameters that
    /// transitive callsite propagation could not resolve, plus enforcement of
    /// the `Numeric` bound itself.
    ///
    /// Runs after `apply_callsite_unions`. For every parameter recorded in
    /// `callable_numeric_param_indices`:
    ///
    /// * still an unresolved variable → collapse to `number` (last resort);
    /// * resolved to a numeric type (or all-numeric union) → keep the precise
    ///   type the call graph produced;
    /// * resolved to a non-numeric type → emit a `ConstraintViolation`. This
    ///   restores the rejection that the eager `Numeric` → `number` collapse
    ///   used to provide for free: a function body like `c = c + 1` imposes
    ///   `Numeric` on `c`, and a call site passing a non-numeric value (e.g.
    ///   an object) is a type error.
    ///
    /// When a parameter is collapsed to `number`, the collapse is propagated
    /// into the function's return type if the return type is that same
    /// parameter variable (`numeric_result_type` returns the operand variable
    /// for `var <op> concrete-numeric`, so a body like `x * 2` makes the
    /// return type share the parameter's variable).
    fn refine_numeric_params_post_callsite(
        &mut self,
        types: &mut HashMap<String, Type>,
    ) -> Vec<TypeError> {
        let mut errors = Vec::new();
        let numeric_indices = self.callable_numeric_param_indices.clone();
        for (function_name, indices) in numeric_indices {
            let Some(Type::Function { params, returns }) = types.get(&function_name) else {
                continue;
            };
            let mut new_params = params.clone();
            let mut new_return = *returns.clone();
            let mut local_subst: HashMap<TypeVar, Type> = HashMap::new();

            for &index in &indices {
                let Some(param_ty) = new_params.get_mut(index) else {
                    continue;
                };
                let resolved = self.solver.unifier().apply_substitutions(param_ty);
                match resolved {
                    Type::Variable(var) | Type::Constrained { var, .. } => {
                        // Genuinely unresolved → apply the `number` default.
                        local_subst.insert(var, BuiltinTypes::number());
                        *param_ty = BuiltinTypes::number();
                    }
                    concrete => {
                        if !Self::type_satisfies_numeric_bound(&concrete) {
                            // Callsite propagation widened a `Numeric`-bounded
                            // parameter to a non-numeric type — a type error.
                            errors.push(TypeError::ConstraintViolation(format!(
                                "parameter at position {} of '{}' must be numeric \
                                 (its body requires a Numeric operand), but a call site \
                                 passes the non-numeric type '{}'",
                                index,
                                function_name,
                                self.render_type_for_diag(&concrete),
                            )));
                        }
                        // Transitive propagation already resolved it — keep
                        // the precise type the call graph produced (even when
                        // it violates the bound, so downstream diagnostics
                        // still see a stable type).
                        *param_ty = concrete;
                    }
                }
            }

            if !local_subst.is_empty() {
                new_return = Self::apply_substitutions_to_type(&new_return, &local_subst);
            }

            types.insert(
                function_name,
                Type::Function {
                    params: new_params,
                    returns: Box::new(new_return),
                },
            );
        }
        errors
    }

    /// ROOT-2 last-resort default for unannotated `Numeric`-bounded CLOSURE
    /// parameters that NO call site ever resolved.
    ///
    /// Closures are not in `callable_numeric_param_indices` (that table is keyed
    /// by named-function symbol), so `refine_numeric_params_post_callsite` does
    /// not touch them. Their param source variables are recorded in
    /// `deferred_closure_numeric_param_vars` by the `Expr::FunctionExpr` arm.
    /// After `solver.solve` has unified every call site, a called closure's
    /// param var is already bound to the concrete argument type (e.g. `int` for
    /// `let f = |x| x * 2; f(i)` where `i: int`); only a NEVER-called closure
    /// leaves the var unresolved. We bind those leftovers to `number` — the same
    /// last-resort default the named-function path applies, and the same default
    /// that the pre-ROOT-2 eager closure collapse provided. Binding flows through
    /// the unifier so the substitution loop propagates it into the closure's
    /// stored function type.
    ///
    /// Soundness: this only fires for a variable that NO concrete argument ever
    /// pinned. It never converts a resolved `int` param to `number` — a bound
    /// var is `Type::Concrete`/`Type::Generic` after `apply_substitutions`, not
    /// `Type::Variable`/`Type::Constrained`, so it is skipped. No int VALUE is
    /// widened.
    /// S1: scan a closure body for `<param> <arith-op> <numeric-literal>` (or
    /// the literal-on-left form) and return the literal's natural numeric type
    /// (`int` for an integer literal, `number` for a float literal). This is
    /// the inference-engine mirror of the bytecode compiler's
    /// `infer_param_type_from_body` body-literal heuristic — it gives a
    /// FORWARDED closure whose param the call graph never pins a sound,
    /// body-proven type (the closure is not genuinely polymorphic). Only
    /// arithmetic / bitwise ops pair the operand by family; comparisons yield
    /// bool and are ignored. A body with no such pairing yields `None` (the var
    /// then stays an honest proof-gap and is rejected by the caller). Returns
    /// the FIRST pairing found in source order — a body that pairs the same
    /// param with conflicting literal families (`|x| x + 1 + 2.0`) is already a
    /// same-family arithmetic error downstream, so first-wins is safe here.
    fn closure_body_literal_param_type(
        param_name: &str,
        body: &[shape_ast::ast::Statement],
    ) -> Option<Type> {
        use shape_ast::ast::{Expr, Literal, Statement};

        fn lit_numeric_type(lit: &Literal) -> Option<Type> {
            match lit {
                Literal::Int(_) => Some(BuiltinTypes::integer()),
                Literal::Number(_) => Some(BuiltinTypes::number()),
                _ => None,
            }
        }

        fn scan_expr(name: &str, expr: &Expr) -> Option<Type> {
            use shape_ast::ast::BinaryOp;
            match expr {
                Expr::BinaryOp {
                    left, op, right, ..
                } => {
                    // Only arithmetic / bitwise ops pair operands numerically.
                    let is_numeric_op = matches!(
                        op,
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
                            | BinaryOp::BitShr
                    );
                    if is_numeric_op {
                        if let (Expr::Identifier(n, _), Expr::Literal(lit, _)) =
                            (left.as_ref(), right.as_ref())
                        {
                            if n == name {
                                if let Some(t) = lit_numeric_type(lit) {
                                    return Some(t);
                                }
                            }
                        }
                        if let (Expr::Literal(lit, _), Expr::Identifier(n, _)) =
                            (left.as_ref(), right.as_ref())
                        {
                            if n == name {
                                if let Some(t) = lit_numeric_type(lit) {
                                    return Some(t);
                                }
                            }
                        }
                    }
                    scan_expr(name, left).or_else(|| scan_expr(name, right))
                }
                Expr::UnaryOp { operand, .. } => scan_expr(name, operand),
                Expr::FunctionCall { args, .. } => args.iter().find_map(|a| scan_expr(name, a)),
                Expr::MethodCall { receiver, args, .. } => scan_expr(name, receiver)
                    .or_else(|| args.iter().find_map(|a| scan_expr(name, a))),
                Expr::Array(elements, _) => elements.iter().find_map(|e| scan_expr(name, e)),
                Expr::Return(Some(e), _) => scan_expr(name, e),
                _ => None,
            }
        }

        fn scan_stmt(name: &str, stmt: &Statement) -> Option<Type> {
            match stmt {
                Statement::Expression(expr, _) => scan_expr(name, expr),
                Statement::Return(Some(e), _) => scan_expr(name, e),
                Statement::VariableDecl(decl, _) => {
                    decl.value.as_ref().and_then(|e| scan_expr(name, e))
                }
                Statement::Assignment(asgn, _) => scan_expr(name, &asgn.value),
                _ => None,
            }
        }

        body.iter().find_map(|s| scan_stmt(param_name, s))
    }

    fn default_unresolved_closure_numeric_params(&mut self) -> Vec<TypeError> {
        let mut errors = Vec::new();
        let vars: Vec<TypeVar> = self
            .deferred_closure_numeric_param_vars
            .iter()
            .cloned()
            .collect();
        for var in vars {
            match self
                .solver
                .unifier()
                .apply_substitutions(&Type::Variable(var.clone()))
            {
                Type::Variable(_) | Type::Constrained { .. } => {
                    // S1: the closure's OWN BODY proves the param type via an
                    // int/number literal pairing (`|x| x * 2` ⇒ int). The
                    // closure is not genuinely polymorphic, so the body literal
                    // IS the proof — bind the var to it instead of rejecting /
                    // number-defaulting. This is exactly the task's "the actual
                    // closure body proves int or number". Only reached when no
                    // call site pinned the var (it is still a Variable here), so
                    // §4 literal-adoption at a real call site is untouched.
                    if let Some(hint) = self
                        .deferred_closure_numeric_param_body_hint
                        .get(&var)
                        .cloned()
                    {
                        self.solver.unifier_mut().bind(var, hint);
                        continue;
                    }
                    // Indirected-callable surface (the recurring unsoundness). A
                    // closure that ESCAPED into a user call (recorded in
                    // `escaping_closure_numeric_param_vars`) but whose numeric
                    // param the call graph never pinned cannot default to
                    // `number`: an `int` value may flow into that slot at runtime
                    // (`applyx(id(|a,b| a*b),6,7)` invokes it with int 6,7), and
                    // `MulNumber` on int bits is the silent int->number widening
                    // CLAUDE.md forbids (42.0 not 42; the int-slot index path
                    // misreads the 4.0 as arr[0]). `int` and `number` do NOT
                    // unify, so REJECT cleanly instead of defaulting.
                    if self.escaping_closure_numeric_param_vars.contains(&var) {
                        errors.push(TypeError::ConstraintViolation(
                            "cannot infer the element/operand type of a closure passed as a \
                             function argument: the closure is invoked indirectly (e.g. \
                             returned from another function or forwarded through a wrapper) so \
                             the type of its numeric parameter cannot be proven at compile \
                             time. Annotate the closure's parameter type (e.g. `|a: int, b: \
                             int| …`) or give the receiving function an explicit function-typed \
                             parameter to disambiguate (strict typing: `int` and `number` do \
                             not unify, so an un-inferable numeric operand cannot default)."
                                .to_string(),
                        ));
                        continue;
                    }
                    // Genuinely never-invoked closure (`let f = |x| x*3`): no value
                    // ever flows through the param, so the last-resort `number`
                    // default is harmless and keeps the binding concrete.
                    self.solver.unifier_mut().bind(var, BuiltinTypes::number());
                }
                _ => {}
            }
        }
        errors
    }

    /// ROOT-B post-solve default: an `Ok`/`Err`/`Some` constructor's
    /// bare-int-literal payload var that DEFERRED (see
    /// `deferred_constructor_literal_payload_vars`) and was NEVER resolved by a
    /// carrier (`let x = Some(42); x` — x used bare, nothing pins `T`) defaults
    /// to `int`, the literal's NATURAL type. Mirrors
    /// `default_unresolved_closure_numeric_params` but uses `int` (not
    /// `number`): the deferred value is an integer literal, so its default
    /// family is `int`. This keeps the binding concrete (`Option<int>` rather
    /// than an un-pinnable `Option<T>`) and introduces no widening — a resolved
    /// var keeps whatever the flow pinned it to (`number` for the
    /// `Result<number>` return class).
    fn default_unresolved_constructor_literal_payload_vars(&mut self) {
        let vars: Vec<TypeVar> = self
            .deferred_constructor_literal_payload_vars
            .iter()
            .cloned()
            .collect();
        for var in vars {
            match self
                .solver
                .unifier()
                .apply_substitutions(&Type::Variable(var.clone()))
            {
                Type::Variable(_) | Type::Constrained { .. } => {
                    self.solver.unifier_mut().bind(var, BuiltinTypes::integer());
                }
                _ => {}
            }
        }
    }

    fn propagate_param_destructure_field_links(&mut self) -> Vec<TypeError> {
        let mut constraints = Vec::new();
        for (scrutinee, elem_var) in self.param_destructure_array_element_links.clone() {
            let resolved_elem_var = self
                .solver
                .unifier()
                .apply_substitutions(&Type::Variable(elem_var.clone()));
            if Self::type_is_fully_resolved(&resolved_elem_var) {
                continue;
            }

            let Some(elem_ty) = self.param_destructure_resolved_array_element_type(&scrutinee)
            else {
                continue;
            };
            constraints.push((Type::Variable(elem_var), elem_ty));
        }

        for (scrutinee, field_name, field_var) in self.param_destructure_field_links.clone() {
            let resolved_field_var = self
                .solver
                .unifier()
                .apply_substitutions(&Type::Variable(field_var.clone()));
            if Self::type_is_fully_resolved(&resolved_field_var) {
                continue;
            }

            let Some(field_ty) =
                self.param_destructure_resolved_field_type(&scrutinee, &field_name)
            else {
                continue;
            };
            constraints.push((Type::Variable(field_var), field_ty));
        }

        if constraints.is_empty() {
            return Vec::new();
        }

        match self.solver.solve(&mut constraints) {
            Ok(()) => Vec::new(),
            Err(err) => vec![err],
        }
    }

    fn param_destructure_resolved_array_element_type(&self, scrutinee: &Type) -> Option<Type> {
        let resolved = self.solver.unifier().apply_substitutions(scrutinee);
        match resolved.canonicalize() {
            Type::Generic { base, args }
                if args.len() == 1
                    && matches!(
                        base.as_ref(),
                        Type::Concrete(TypeAnnotation::Reference(name))
                            if name.as_str() == "Array" || name.as_str() == "Vec"
                    ) =>
            {
                args.into_iter().next()
            }
            Type::Concrete(TypeAnnotation::Array(elem)) => {
                Some(Self::callsite_type_from_annotation_preserving_tyvars(&elem))
            }
            _ => None,
        }
    }

    fn param_destructure_resolved_field_type(
        &self,
        scrutinee: &Type,
        field_name: &str,
    ) -> Option<Type> {
        let resolved = self.solver.unifier().apply_substitutions(scrutinee);
        match &resolved {
            Type::Concrete(TypeAnnotation::Object(fields)) => fields
                .iter()
                .find(|field| field.name == field_name)
                .map(|field| {
                    Self::callsite_type_from_annotation_preserving_tyvars(&field.type_annotation)
                }),
            _ => {
                let struct_name = self
                    .struct_name_of_type(&resolved)
                    .or_else(|| self.struct_name_of_type(scrutinee))?;
                self.struct_field_annotation(&struct_name, field_name)
                    .map(|ann| self.resolve_type_annotation(&ann))
            }
        }
    }

    /// Infer a single expression's type AND finalize it the way `infer_program`
    /// does for the whole-program pass: solve the accumulated constraints,
    /// ground any DEFERRED `Ok`/`Err`/`Some` bare-int-literal payload var to its
    /// natural `int` (the ROOT-B post-solve default), then apply substitutions.
    ///
    /// Bare `infer_expr` leaves the constraints unsolved and never runs the
    /// post-solve default pass, so a `let r = Ok(1)?` payload var flows out as an
    /// unresolved `Type::Variable` (rendered `T`) — wrong for the LSP inlay/hover
    /// display. This finalized variant resolves it to `int`, matching what the
    /// program-level inference would record. Used by LSP single-expr display
    /// inference; it does NOT clear engine state, so callers should use a fresh
    /// engine per expression (as the LSP helpers already do).
    pub fn infer_expr_finalized(&mut self, expr: &shape_ast::ast::Expr) -> TypeResult<Type> {
        let ty = self.infer_expr(expr)?;
        // Solve the constraints this expression accumulated. SB-2: the solver's
        // unifier is the single store, so the deferred-var grounding + final
        // `apply_substitutions` read those bindings directly.
        let _ = self.solver.solve(&mut self.constraints);
        self.default_unresolved_constructor_literal_payload_vars();
        Ok(self.solver.unifier().apply_substitutions(&ty))
    }

    fn propagate_return_alias_substitution(
        &self,
        return_type: Type,
        substitutions: &mut HashMap<TypeVar, Type>,
    ) {
        let Type::Variable(mut current_var) = return_type else {
            return;
        };
        if substitutions.contains_key(&current_var) {
            return;
        }

        let mut visited: HashSet<TypeVar> = HashSet::new();
        while visited.insert(current_var.clone()) {
            let Some(alias_var) = self.return_var_aliases.get(&current_var).cloned() else {
                break;
            };

            if let Some(alias_subst) = substitutions.get(&alias_var).cloned() {
                substitutions.insert(current_var.clone(), alias_subst);
                break;
            }

            current_var = alias_var;
        }
    }

    fn materialize_pending_return_union(
        &mut self,
        return_type: Type,
        substitutions: &HashMap<TypeVar, Type>,
    ) -> Type {
        let substituted_return = Self::apply_substitutions_to_type(&return_type, substitutions);
        let base_var = match &return_type {
            Type::Variable(var) => Some(var),
            _ => None,
        };
        let Some(base_var) = base_var else {
            return substituted_return;
        };
        let Some(pending_members) = self.pending_return_unions.get(base_var).cloned() else {
            return substituted_return;
        };

        let mut members: Vec<Type> = Vec::new();
        if !matches!(substituted_return, Type::Variable(_)) {
            members.push(substituted_return.clone());
        }

        for member in pending_members {
            let resolved_member = Self::apply_substitutions_to_type(&member, substitutions);
            if matches!(resolved_member, Type::Variable(_)) {
                continue;
            }
            if !members
                .iter()
                .any(|existing| self.types_equal(existing, &resolved_member))
            {
                members.push(resolved_member);
            }
        }

        match members.len() {
            0 => substituted_return,
            1 => members.into_iter().next().unwrap_or(substituted_return),
            _ => self.create_nominal_union(&members).unwrap_or_else(|_| {
                let variants: Vec<TypeAnnotation> =
                    members.iter().filter_map(|t| t.to_annotation()).collect();
                Type::Concrete(TypeAnnotation::Union(variants))
            }),
        }
    }

    /// Resolve a type through the unifier and the transitive callsite-union
    /// `resolved` map until it is concrete or no further substitution applies.
    /// Used by the fixpoint in `apply_callsite_unions` so a still-`Type::Variable`
    /// observed argument resolves once some other function in the call graph
    /// has been concretized.
    ///
    /// Both maps are consulted at each step: the unifier and `resolved` key
    /// off different variables (the unifier off the constraint-solver's
    /// variables; `resolved` off each function's recorded parameter-source /
    /// return variables), and either may rename a variable into the other's
    /// key space. Checking both per step — `resolved` first, then the unifier
    /// — chases the chain regardless of which map owns the next hop.
    fn resolve_through_callsite_map(&self, ty: &Type, resolved: &HashMap<TypeVar, Type>) -> Type {
        let mut current = ty.clone();
        // Bound: every step must consume one variable from a finite pool, so
        // the unifier's variable count plus the `resolved` size is a safe cap.
        let max_steps = self.solver.unifier().substitutions().len() + resolved.len() + 2;
        for _ in 0..max_steps {
            let Type::Variable(var) = &current else {
                break;
            };
            // Transitive callsite-union hop.
            if let Some(next) = resolved.get(var) {
                let stepped = next.clone();
                if stepped == current {
                    break;
                }
                current = stepped;
                continue;
            }
            // Constraint-solver hop.
            let stepped = self.solver.unifier().apply_substitutions(&current);
            if stepped == current {
                break;
            }
            current = stepped;
        }
        current
    }

    fn union_from_observed_types_with_resolved(
        &mut self,
        observed_types: &[Type],
        resolved: &HashMap<TypeVar, Type>,
    ) -> Option<Type> {
        let mut unique = Vec::new();
        for ty in observed_types {
            let normalized = self.resolve_through_callsite_map(ty, resolved);
            if matches!(normalized, Type::Variable(_)) {
                continue;
            }
            if !unique
                .iter()
                .any(|existing| self.types_equal(existing, &normalized))
            {
                unique.push(normalized);
            }
        }

        match unique.len() {
            0 => None,
            1 => unique.into_iter().next(),
            _ => self.create_nominal_union(&unique).ok().or_else(|| {
                let variants: Vec<TypeAnnotation> =
                    unique.iter().filter_map(|t| t.to_annotation()).collect();
                Some(Type::Concrete(TypeAnnotation::Union(variants)))
            }),
        }
    }

    fn callsite_type_from_annotation_preserving_tyvars(ann: &TypeAnnotation) -> Type {
        if let Some(var) = annotation_as_tyvar(ann) {
            return Type::Variable(var);
        }

        match ann {
            TypeAnnotation::Array(elem) => {
                BuiltinTypes::array(Self::callsite_type_from_annotation_preserving_tyvars(elem))
            }
            TypeAnnotation::Generic { name, args } => Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference(name.clone()))),
                args: args
                    .iter()
                    .map(Self::callsite_type_from_annotation_preserving_tyvars)
                    .collect(),
            },
            TypeAnnotation::Function { params, returns } => Type::Function {
                params: params
                    .iter()
                    .map(|param| {
                        Self::callsite_type_from_annotation_preserving_tyvars(
                            &param.type_annotation,
                        )
                    })
                    .collect(),
                returns: Box::new(Self::callsite_type_from_annotation_preserving_tyvars(
                    returns,
                )),
            },
            TypeAnnotation::Object(fields) => Type::Concrete(TypeAnnotation::Object(
                fields
                    .iter()
                    .map(|field| ObjectTypeField {
                        name: field.name.clone(),
                        optional: field.optional,
                        type_annotation: field.type_annotation.clone(),
                        annotations: field.annotations.clone(),
                    })
                    .collect(),
            )),
            other => Type::Concrete(other.clone()),
        }
    }

    fn apply_substitutions_to_type(ty: &Type, substitutions: &HashMap<TypeVar, Type>) -> Type {
        match ty {
            Type::Variable(var) => substitutions
                .get(var)
                .cloned()
                .unwrap_or_else(|| ty.clone()),
            Type::Generic { base, args } => Type::Generic {
                base: Box::new(Self::apply_substitutions_to_type(base, substitutions)),
                args: args
                    .iter()
                    .map(|arg| Self::apply_substitutions_to_type(arg, substitutions))
                    .collect(),
            },
            Type::Constrained { var, constraint } => substitutions
                .get(var)
                .cloned()
                .unwrap_or_else(|| Type::Constrained {
                    var: var.clone(),
                    constraint: constraint.clone(),
                }),
            Type::Function { params, returns } => Type::Function {
                params: params
                    .iter()
                    .map(|param| Self::apply_substitutions_to_type(param, substitutions))
                    .collect(),
                returns: Box::new(Self::apply_substitutions_to_type(returns, substitutions)),
            },
            // A concrete annotation can still embed `tyvar` markers — an
            // object literal `{min: lo}` over an unresolved parameter freezes
            // to `Object({min: <tyvar lo>})`. Recurse so callsite
            // substitution resolves those embedded markers.
            Type::Concrete(ann) => {
                Type::Concrete(Self::apply_substitutions_to_annotation(ann, substitutions))
            }
        }
    }

    /// Recurse through a `TypeAnnotation`, replacing every `tyvar` marker with
    /// its bound type (re-encoding as a marker if the binding is itself an
    /// unresolved variable). Mirrors `Unifier::apply_to_annotation` but keys
    /// off an explicit substitution map rather than the unifier's store.
    fn apply_substitutions_to_annotation(
        ann: &TypeAnnotation,
        substitutions: &HashMap<TypeVar, Type>,
    ) -> TypeAnnotation {
        if let Some(var) = annotation_as_tyvar(ann) {
            return match substitutions.get(&var) {
                Some(bound) => {
                    let resolved = Self::apply_substitutions_to_type(bound, substitutions);
                    match &resolved {
                        Type::Variable(v) | Type::Constrained { var: v, .. } => {
                            tyvar_to_annotation(v)
                        }
                        _ => resolved.to_annotation().unwrap_or_else(|| ann.clone()),
                    }
                }
                None => ann.clone(),
            };
        }
        match ann {
            TypeAnnotation::Borrow { mutable, inner } => TypeAnnotation::Borrow {
                mutable: *mutable,
                inner: Box::new(Self::apply_substitutions_to_annotation(
                    inner,
                    substitutions,
                )),
            },
            TypeAnnotation::Array(elem) => TypeAnnotation::Array(Box::new(
                Self::apply_substitutions_to_annotation(elem, substitutions),
            )),
            TypeAnnotation::Tuple(elems) => TypeAnnotation::Tuple(
                elems
                    .iter()
                    .map(|e| Self::apply_substitutions_to_annotation(e, substitutions))
                    .collect(),
            ),
            TypeAnnotation::Object(fields) => TypeAnnotation::Object(
                fields
                    .iter()
                    .map(|field| shape_ast::ast::ObjectTypeField {
                        name: field.name.clone(),
                        optional: field.optional,
                        type_annotation: Self::apply_substitutions_to_annotation(
                            &field.type_annotation,
                            substitutions,
                        ),
                        annotations: field.annotations.clone(),
                    })
                    .collect(),
            ),
            TypeAnnotation::Function { params, returns } => TypeAnnotation::Function {
                params: params
                    .iter()
                    .map(|p| shape_ast::ast::FunctionParam {
                        name: p.name.clone(),
                        optional: p.optional,
                        type_annotation: Self::apply_substitutions_to_annotation(
                            &p.type_annotation,
                            substitutions,
                        ),
                    })
                    .collect(),
                returns: Box::new(Self::apply_substitutions_to_annotation(
                    returns,
                    substitutions,
                )),
            },
            TypeAnnotation::Union(members) => TypeAnnotation::Union(
                members
                    .iter()
                    .map(|m| Self::apply_substitutions_to_annotation(m, substitutions))
                    .collect(),
            ),
            TypeAnnotation::Intersection(members) => TypeAnnotation::Intersection(
                members
                    .iter()
                    .map(|m| Self::apply_substitutions_to_annotation(m, substitutions))
                    .collect(),
            ),
            TypeAnnotation::Generic { name, args } => TypeAnnotation::Generic {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| Self::apply_substitutions_to_annotation(a, substitutions))
                    .collect(),
            },
            TypeAnnotation::Basic(_)
            | TypeAnnotation::Reference(_)
            | TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined
            | TypeAnnotation::Dyn(_) => ann.clone(),
        }
    }
}

#[cfg(test)]
#[path = "inference_tests.rs"]
mod tests;
