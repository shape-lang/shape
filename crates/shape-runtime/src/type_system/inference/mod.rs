//! Type Inference Engine
//!
//! Implements Hindley-Milner style type inference with extensions
//! for Shape's domain-specific features.

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
            pending_return_unions: HashMap::new(),
            return_var_aliases: HashMap::new(),
            return_scopes: Vec::new(),
            implicit_return_scopes: Vec::new(),
            struct_type_defs: HashMap::new(),
        }
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
                self.env
                    .define(name, TypeScheme::mono(Type::Variable(TypeVar::fresh())));
            }
        }
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
                Type::Concrete(TypeAnnotation::Reference(name))
                    | Type::Concrete(TypeAnnotation::Basic(name))
                    if name == "Result"
            ),
            Type::Concrete(TypeAnnotation::Generic { name, .. }) => name == "Result",
            _ => false,
        }
    }

    pub(crate) fn wrap_result_type(&self, inner: Type) -> Type {
        self.wrap_result_type_with_error(inner, self.any_error_type())
    }

    pub(crate) fn wrap_result_type_with_error(&self, inner: Type, err: Type) -> Type {
        Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Result".to_string(),
            ))),
            args: vec![inner, err],
        }
    }

    pub(crate) fn any_error_type(&self) -> Type {
        Type::Concrete(TypeAnnotation::Reference("AnyError".to_string()))
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
    pub(crate) fn refine_callable_param_types_from_local_constraints(
        &self,
        param_types: &mut [Type],
        local_constraints: &[(Type, Type)],
        include_numeric_refinement: bool,
    ) {
        for param_type in param_types.iter_mut() {
            let Type::Variable(param_var) = param_type else {
                continue;
            };

            if let Some(fields) =
                self.project_object_param_fields_from_constraints(param_var, local_constraints)
            {
                *param_type = Type::Concrete(TypeAnnotation::Object(fields));
                continue;
            }

            if include_numeric_refinement
                && self.var_has_constraint(local_constraints, param_var, |constraint| {
                    matches!(constraint, TypeConstraint::Numeric)
                })
            {
                *param_type = BuiltinTypes::number();
            }
        }
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
            matches!(constraint, TypeConstraint::Numeric)
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
            TypeAnnotation::Reference(name) => name.clone(),
            TypeAnnotation::Array(_) => "array".to_string(),
            TypeAnnotation::Object(_) => "object".to_string(),
            TypeAnnotation::Function { .. } => "function".to_string(),
            TypeAnnotation::Union(_) => "union".to_string(),
            TypeAnnotation::Optional(_) => "optional".to_string(),
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
                        Type::Concrete(TypeAnnotation::Reference(name))
                        | Type::Concrete(TypeAnnotation::Basic(name)) => name.clone(),
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

    fn as_generic_components(&self, ty: &Type) -> Option<(Type, Vec<Type>)> {
        match ty {
            Type::Generic { base, args } => {
                let mut normalized_args = args.clone();
                if matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name))
                        | Type::Concrete(TypeAnnotation::Basic(name))
                        if name == "Result"
                ) && normalized_args.len() == 1
                {
                    normalized_args.push(Type::Variable(TypeVar::fresh()));
                }
                Some(((*base.clone()), normalized_args))
            }
            Type::Concrete(TypeAnnotation::Generic { name, args }) => {
                let mut normalized_args = args
                    .iter()
                    .map(|arg| Type::Concrete(arg.clone()))
                    .collect::<Vec<_>>();
                if name == "Result" && normalized_args.len() == 1 {
                    normalized_args.push(Type::Variable(TypeVar::fresh()));
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
                    let representative = unresolved_candidates
                        .first()
                        .cloned()
                        .unwrap_or_else(|| Type::Variable(TypeVar::fresh()));
                    for unresolved in unresolved_candidates.iter().skip(1) {
                        self.constraints
                            .push((representative.clone(), unresolved.clone()));
                    }
                    merged_args.push(representative);
                    continue;
                }

                let base_name = match &base {
                    Type::Concrete(TypeAnnotation::Reference(name))
                    | Type::Concrete(TypeAnnotation::Basic(name)) => name.clone(),
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

        // Apply substitutions to get final types
        for (_name, ty) in types.iter_mut() {
            *ty = self.unifier.apply_substitutions(ty);
        }

        (types, errors)
    }

    /// Rebuild callable default-parameter metadata from the semantic symbol table.
    ///
    /// This is used by legacy semantic analysis paths that infer expressions
    /// incrementally instead of running whole-program inference.
    pub fn sync_callable_defaults_from_symbol_table(
        &mut self,
        symbol_table: &crate::semantic::symbol_table::SymbolTable,
    ) {
        self.callable_param_defaults.clear();
        Self::seed_builtin_callable_defaults(&mut self.callable_param_defaults);

        for (name, symbol) in symbol_table.iter_all_symbols() {
            if let crate::semantic::symbol_table::Symbol::Function { defaults, .. } = symbol {
                self.callable_param_defaults
                    .insert(name.to_string(), defaults.clone());
            }
        }
    }

    fn apply_callsite_unions(&mut self, types: &mut HashMap<String, Type>) {
        let callsites = self.callsite_param_types.clone();
        for (function_name, observed_by_param) in callsites {
            let Some(Type::Function { params, returns }) = types.get(&function_name) else {
                continue;
            };
            let param_source_vars = self
                .callable_param_source_vars
                .get(&function_name)
                .cloned()
                .unwrap_or_default();

            let mut widened_params = params.clone();
            let mut substitutions: HashMap<TypeVar, Type> = HashMap::new();

            for (index, observed_types) in observed_by_param.iter().enumerate() {
                if index >= widened_params.len() {
                    break;
                }
                let Some(widened_type) = self.union_from_observed_types(observed_types) else {
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

            self.propagate_return_alias_substitution(*returns.clone(), &mut substitutions);
            let widened_return =
                self.materialize_pending_return_union(*returns.clone(), &substitutions);

            types.insert(
                function_name,
                Type::Function {
                    params: widened_params,
                    returns: Box::new(widened_return),
                },
            );
        }
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
                let variants: Vec<TypeAnnotation> = members
                    .iter()
                    .filter_map(|t| t.to_annotation())
                    .collect();
                Type::Concrete(TypeAnnotation::Union(variants))
            }),
        }
    }

    fn union_from_observed_types(&mut self, observed_types: &[Type]) -> Option<Type> {
        let mut unique = Vec::new();
        for ty in observed_types {
            let normalized = self.unifier.apply_substitutions(ty);
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
                let variants: Vec<TypeAnnotation> = unique
                    .iter()
                    .filter_map(|t| t.to_annotation())
                    .collect();
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
            Type::Concrete(_) => ty.clone(),
        }
    }
}

#[cfg(test)]
#[path = "inference_tests.rs"]
mod tests;
