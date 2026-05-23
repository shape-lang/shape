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
use super::unification::Unifier;
use super::*;
use shape_ast::ast::{ObjectTypeField, Program, Span, StructTypeDef, TypeAnnotation};
use std::collections::HashMap;

use crate::type_system::semantic::{EnumVariant, SemanticType};
use std::collections::HashSet;

pub struct TypeInferenceEngine {
    /// Type environment tracking variable types
    pub env: TypeEnvironment,
    /// Per-engine generator for fresh type variables (B4). Replaces the
    /// former process-global `NEXT_TYPEVAR_ID` counter so test runs and
    /// independent inference sessions can't alias each other's IDs.
    pub type_var_gen: crate::type_system::TypeVarGen,
    /// Constraint solver for type constraints
    pub(crate) solver: ConstraintSolver,
    /// Type unifier
    pub(crate) unifier: Unifier,
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
    /// Source type variables for callable parameters, indexed by parameter
    /// position. `None` means parameter was explicitly annotated.
    pub(crate) callable_param_source_vars: HashMap<String, Vec<Option<TypeVar>>>,
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
    /// Struct type definitions keyed by name for generic struct-literal inference.
    pub(crate) struct_type_defs: HashMap<String, StructTypeDef>,
    /// Resolved type parameter substitutions at generic call sites.
    /// Key: (function_name, span_start, span_end)
    /// Value: [(original_param_name, concrete_TypeAnnotation)]
    ///
    /// Populated during `infer_function_call` when all type params of a
    /// polymorphic callee resolve to concrete types. Consumed by the
    /// bytecode compiler to drive monomorphization.
    pub callsite_type_args:
        HashMap<(String, usize, usize), Vec<(String, TypeAnnotation)>>,
    /// J-CT.1: depth of the current `comptime { ... }` nesting.
    ///
    /// Incremented on entering `Expr::Comptime` / `Expr::ComptimeFor` /
    /// `Item::Comptime` during inference; decremented on exit. The
    /// method-call type-checker (`expressions.rs::Expr::MethodCall`)
    /// rejects calls to `comptime impl`-registered methods when this
    /// counter is zero. A counter (not a bool) keeps nested comptime
    /// blocks correct, but in practice the depth stays at 0 or 1.
    pub(crate) comptime_depth: usize,
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
        // round(value), round(value, decimals)
        defaults.insert("round".to_string(), vec![false, true]);
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
            unifier: Unifier::new(),
            constraints: Vec::new(),
            constraint_origins: HashMap::new(),
            callable_origins_by_name: HashMap::new(),
            unknown_property_origins: HashMap::new(),
            undefined_variable_origins: HashMap::new(),
            non_exhaustive_match_origins: HashMap::new(),
            fallible_scopes: Vec::new(),
            method_table: MethodTable::new(),
            callsite_param_types: HashMap::new(),
            callable_param_source_vars: HashMap::new(),
            callable_param_defaults,
            callable_numeric_param_indices: HashMap::new(),
            pending_return_unions: HashMap::new(),
            return_var_aliases: HashMap::new(),
            return_scopes: Vec::new(),
            implicit_return_scopes: Vec::new(),
            struct_type_defs: HashMap::new(),
            callsite_type_args: HashMap::new(),
            comptime_depth: 0,
        }
    }

    /// J-CT.1: enter a comptime context (e.g. `comptime { ... }`).
    pub(crate) fn enter_comptime(&mut self) {
        self.comptime_depth = self.comptime_depth.saturating_add(1);
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

    pub(crate) fn record_pending_return_union(
        &mut self,
        base_var: TypeVar,
        additional_members: impl IntoIterator<Item = Type>,
    ) {
        let entry = self
            .pending_return_unions
            .entry(base_var)
            .or_insert_with(Vec::new);
        for member in additional_members {
            if !entry
                .iter()
                .any(|existing| crate::type_system::unification::types_equal(existing, &member))
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

    pub(crate) fn wrap_result_type(&self, inner: Type) -> Type {
        self.wrap_result_type_with_error(inner, self.any_error_type())
    }

    pub(crate) fn wrap_result_type_with_error(&self, inner: Type, err: Type) -> Type {
        Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Result".into(),
            ))),
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
        if is_fallible && !self.is_result_type(&return_ty) {
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
                if is_closure {
                    *param_type = BuiltinTypes::number();
                }
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

    /// Check if two types are structurally equal
    ///
    /// Uses proper structural equality instead of string-based comparison.
    fn types_equal(&self, a: &Type, b: &Type) -> bool {
        crate::type_system::unification::types_equal(a, b)
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
        self.callable_param_source_vars.clear();
        self.callable_param_defaults.clear();
        self.callable_numeric_param_indices.clear();
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
        // Run hoisting pre-pass first
        self.run_hoisting_prepass(program);

        let mut types = HashMap::new();
        let mut errors = Vec::new();

        // First pass: predeclare callable symbols/methods so references are
        // order-independent (matches compiler front-end behavior).
        for item in &program.items {
            if let Err(err) = self.predeclare_item(item) {
                errors.push(err);
            }
        }

        for item in &program.items {
            if let Err(err) = self.infer_item(item, &mut types) {
                errors.push(err);
            }
        }

        // Attach the method table and trait impl data to the solver,
        // then solve all constraints
        self.solver.set_method_table(self.method_table.clone());
        self.solver.set_trait_impls(self.env.trait_impl_keys());
        if let Err(err) = self.solver.solve(&mut self.constraints) {
            errors.push(err);
        }
        self.unifier.merge(self.solver.unifier());

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

        // Apply substitutions to get final types
        for (_name, ty) in types.iter_mut() {
            *ty = self.unifier.apply_substitutions(ty);
        }

        (types, errors)
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

                self.propagate_return_alias_substitution(returns.clone(), &mut substitutions);
                let widened_return =
                    self.materialize_pending_return_union(returns.clone(), &substitutions);

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
        for (var, ty) in &resolved {
            if matches!(ty, Type::Variable(_)) {
                continue;
            }
            match self.unifier.lookup(var) {
                Some(existing) if !matches!(existing, Type::Variable(_)) => {
                    // Already concretely bound — leave the prior binding;
                    // a real conflict is reported by constraint solving.
                }
                _ => self.unifier.bind(var.clone(), ty.clone()),
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
            Type::Concrete(TypeAnnotation::Union(members)) => {
                members.iter().all(name_is_numeric)
            }
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
                let resolved = self.unifier.apply_substitutions(param_ty);
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
        let max_steps = self.unifier.substitutions().len() + resolved.len() + 2;
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
            let stepped = self.unifier.apply_substitutions(&current);
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
                        _ => resolved
                            .to_annotation()
                            .unwrap_or_else(|| ann.clone()),
                    }
                }
                None => ann.clone(),
            };
        }
        match ann {
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
                returns: Box::new(Self::apply_substitutions_to_annotation(returns, substitutions)),
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
