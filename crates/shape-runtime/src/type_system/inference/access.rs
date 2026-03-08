//! Access pattern type inference
//!
//! Handles type inference for property access, index access, function calls, and iterators.

use super::TypeInferenceEngine;
use crate::type_system::*;
use shape_ast::ast::{Expr, Span, TypeAnnotation};
use std::collections::HashMap;

impl TypeInferenceEngine {
    /// Infer type of property access
    pub(crate) fn infer_property_access(
        &mut self,
        object_type: &Type,
        property: &str,
    ) -> TypeResult<Type> {
        self.infer_property_access_internal(object_type, property, false)
    }

    /// Infer type of a property assignment target.
    ///
    /// Unlike reads, assignment targets may reference hoisted fields before first write.
    pub(crate) fn infer_property_assignment_target(
        &mut self,
        object_type: &Type,
        property: &str,
    ) -> TypeResult<Type> {
        self.infer_property_access_internal(object_type, property, true)
    }

    fn infer_property_access_internal(
        &mut self,
        object_type: &Type,
        property: &str,
        assignment_target: bool,
    ) -> TypeResult<Type> {
        if let Type::Concrete(TypeAnnotation::Reference(name)) = object_type {
            // Check struct type definitions FIRST (includes comptime fields),
            // before type aliases (which only contain runtime fields).
            if let Some(struct_def) = self.struct_type_defs.get(name.as_str()).cloned() {
                for field in &struct_def.fields {
                    if field.name == property {
                        return Ok(Type::Concrete(field.type_annotation.clone()));
                    }
                }
                return Err(TypeError::UnknownProperty(
                    name.clone(),
                    property.to_string(),
                ));
            }
            // Fall back to type alias resolution
            if let Some(alias_entry) = self.env.lookup_type_alias(name) {
                return self.infer_property_access_internal(
                    &Type::Concrete(alias_entry.type_annotation.clone()),
                    property,
                    assignment_target,
                );
            }
        }

        // Special handling for known types
        match object_type {
            Type::Generic { base, args } => {
                if let Some(type_name) = Self::generic_base_name(base) {
                    if type_name == "Table" {
                        return self.infer_property_access_fallback(object_type, property);
                    }
                    if type_name == "Row" && args.len() == 1 {
                        return self.infer_property_access_internal(
                            &args[0],
                            property,
                            assignment_target,
                        );
                    }

                    if let Some(field_type) =
                        self.resolve_struct_generic_field(type_name, args, property)
                    {
                        return Ok(field_type);
                    }

                    if self.struct_type_defs.contains_key(type_name) {
                        return Err(TypeError::UnknownProperty(
                            type_name.to_string(),
                            property.to_string(),
                        ));
                    }
                }

                self.infer_property_access_fallback(object_type, property)
            }
            // Row<T> property access: resolve field against T's schema
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if name == "Row" && args.len() == 1 =>
            {
                // Extract the inner type T and resolve the property on T
                let inner_type = Type::Concrete(args[0].clone());
                self.infer_property_access_internal(&inner_type, property, assignment_target)
            }
            // Table<T> property access: delegate to DataTable methods
            Type::Concrete(TypeAnnotation::Generic { name, .. }) if name == "Table" => {
                self.infer_property_access_fallback(object_type, property)
            }
            Type::Concrete(TypeAnnotation::Generic { name, args }) => {
                let generic_args: Vec<Type> = args.iter().cloned().map(Type::Concrete).collect();
                if let Some(field_type) =
                    self.resolve_struct_generic_field(name, &generic_args, property)
                {
                    return Ok(field_type);
                }
                if self.struct_type_defs.contains_key(name) {
                    return Err(TypeError::UnknownProperty(
                        name.clone(),
                        property.to_string(),
                    ));
                }
                self.infer_property_access_fallback(object_type, property)
            }
            // Check if this is a registered record schema type
            Type::Concrete(TypeAnnotation::Basic(name)) => {
                // Look up record schema from environment
                if let Some(field_type) = self.env.get_record_field_type(name, property) {
                    return Ok(Type::Concrete(field_type.clone()));
                }
                // If schema exists but field doesn't, that's an error
                if self.env.lookup_record_schema(name).is_some() {
                    return Err(TypeError::UnknownProperty(
                        name.clone(),
                        property.to_string(),
                    ));
                }
                // Fall through to other cases for non-schema types
                self.infer_property_access_fallback(object_type, property)
            }
            Type::Concrete(TypeAnnotation::Intersection(types)) => {
                for ty in types {
                    if let Ok(field_type) = self.infer_property_access_internal(
                        &Type::Concrete(ty.clone()),
                        property,
                        assignment_target,
                    ) {
                        return Ok(field_type);
                    }
                }
                Err(TypeError::UnknownProperty(
                    "intersection".to_string(),
                    property.to_string(),
                ))
            }
            Type::Concrete(TypeAnnotation::Object(fields)) => {
                // Object type with known fields - check declared fields first
                if let Some(field) = fields.iter().find(|f| f.name == property) {
                    return Ok(Type::Concrete(field.type_annotation.clone()));
                }

                // Check hoisted fields (from optimistic hoisting pre-pass).
                // Read access requires prior initialization; assignment targets do not.
                let hoisted_type = if assignment_target {
                    self.env.get_hoisted_field_for_assignment(property)
                } else {
                    self.env.get_hoisted_field(property)
                };
                if let Some(hoisted_type) = hoisted_type {
                    return Ok(hoisted_type);
                }

                // Field not found in declared or hoisted fields
                Err(TypeError::UnknownProperty(
                    "object".to_string(),
                    property.to_string(),
                ))
            }
            _ => {
                // If this is a tracked variable with hoisted fields, resolve from hoisting
                // even when the base type is still a type variable.
                let hoisted_type = if assignment_target {
                    self.env.get_hoisted_field_for_assignment(property)
                } else {
                    self.env.get_hoisted_field(property)
                };
                if let Some(hoisted_type) = hoisted_type {
                    return Ok(hoisted_type);
                }

                // Field was hoisted for assignment but not yet initialized for reads.
                if !assignment_target
                    && self
                        .env
                        .get_hoisted_field_for_assignment(property)
                        .is_some()
                {
                    return Err(TypeError::UnknownProperty(
                        "object".to_string(),
                        property.to_string(),
                    ));
                }

                // For unknown types, create a constraint
                let result_type = Type::Variable(TypeVar::fresh());
                let var = TypeVar::fresh();

                self.constraints.push((
                    object_type.clone(),
                    Type::Constrained {
                        var,
                        constraint: Box::new(TypeConstraint::HasField(
                            property.to_string(),
                            Box::new(result_type.clone()),
                        )),
                    },
                ));

                Ok(result_type)
            }
        }
    }

    fn generic_base_name(base: &Type) -> Option<&str> {
        match base {
            Type::Concrete(TypeAnnotation::Reference(name))
            | Type::Concrete(TypeAnnotation::Basic(name)) => Some(name.as_str()),
            _ => None,
        }
    }

    fn resolve_struct_generic_field(
        &self,
        type_name: &str,
        args: &[Type],
        property: &str,
    ) -> Option<Type> {
        let struct_def = self.struct_type_defs.get(type_name)?;
        let field = struct_def
            .fields
            .iter()
            .filter(|f| !f.is_comptime)
            .find(|f| f.name == property)?;
        let type_params = struct_def.type_params.as_ref()?;
        if type_params.is_empty() {
            return Some(Type::Concrete(field.type_annotation.clone()));
        }

        let mut bindings: HashMap<String, TypeAnnotation> = HashMap::new();
        for (tp, arg) in type_params.iter().zip(args.iter()) {
            if let Some(arg_ann) = arg.to_annotation() {
                bindings.insert(tp.name.clone(), arg_ann);
            }
        }
        let resolved =
            Self::substitute_type_params_in_annotation(&field.type_annotation, &bindings);
        Some(Type::Concrete(resolved))
    }

    fn substitute_type_params_in_annotation(
        annotation: &TypeAnnotation,
        bindings: &HashMap<String, TypeAnnotation>,
    ) -> TypeAnnotation {
        match annotation {
            TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => bindings
                .get(name)
                .cloned()
                .unwrap_or_else(|| annotation.clone()),
            TypeAnnotation::Array(inner) => TypeAnnotation::Array(Box::new(
                Self::substitute_type_params_in_annotation(inner, bindings),
            )),
            TypeAnnotation::Tuple(items) => TypeAnnotation::Tuple(
                items
                    .iter()
                    .map(|item| Self::substitute_type_params_in_annotation(item, bindings))
                    .collect(),
            ),
            TypeAnnotation::Object(fields) => TypeAnnotation::Object(
                fields
                    .iter()
                    .map(|field| shape_ast::ast::ObjectTypeField {
                        name: field.name.clone(),
                        optional: field.optional,
                        type_annotation: Self::substitute_type_params_in_annotation(
                            &field.type_annotation,
                            bindings,
                        ),
                        annotations: vec![],
                    })
                    .collect(),
            ),
            TypeAnnotation::Function { params, returns } => TypeAnnotation::Function {
                params: params
                    .iter()
                    .map(|param| shape_ast::ast::FunctionParam {
                        name: param.name.clone(),
                        optional: param.optional,
                        type_annotation: Self::substitute_type_params_in_annotation(
                            &param.type_annotation,
                            bindings,
                        ),
                    })
                    .collect(),
                returns: Box::new(Self::substitute_type_params_in_annotation(
                    returns, bindings,
                )),
            },
            TypeAnnotation::Union(types) => TypeAnnotation::Union(
                types
                    .iter()
                    .map(|ty| Self::substitute_type_params_in_annotation(ty, bindings))
                    .collect(),
            ),
            TypeAnnotation::Intersection(types) => TypeAnnotation::Intersection(
                types
                    .iter()
                    .map(|ty| Self::substitute_type_params_in_annotation(ty, bindings))
                    .collect(),
            ),
            TypeAnnotation::Optional(inner) => TypeAnnotation::Optional(Box::new(
                Self::substitute_type_params_in_annotation(inner, bindings),
            )),
            TypeAnnotation::Generic { name, args } => TypeAnnotation::Generic {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| Self::substitute_type_params_in_annotation(arg, bindings))
                    .collect(),
            },
            TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined
            | TypeAnnotation::Dyn(_) => annotation.clone(),
        }
    }

    /// Fallback for property access when record schema doesn't apply
    fn infer_property_access_fallback(
        &mut self,
        object_type: &Type,
        property: &str,
    ) -> TypeResult<Type> {
        // For unknown types, create a constraint
        let result_type = Type::Variable(TypeVar::fresh());
        let var = TypeVar::fresh();

        self.constraints.push((
            object_type.clone(),
            Type::Constrained {
                var,
                constraint: Box::new(TypeConstraint::HasField(
                    property.to_string(),
                    Box::new(result_type.clone()),
                )),
            },
        ));

        Ok(result_type)
    }

    /// Infer type of index access
    pub(crate) fn infer_index_access(
        &mut self,
        object_type: &Type,
        index_type: &Type,
    ) -> TypeResult<Type> {
        match object_type {
            // Row<T> disallows dynamic string indexing - use row.field instead
            Type::Concrete(TypeAnnotation::Generic { name, .. }) if name == "Row" => {
                Err(TypeError::TypeMismatch(
                    "static field access (row.field)".to_string(),
                    "dynamic index access (row[...]) on typed Row<T>".to_string(),
                ))
            }
            Type::Concrete(TypeAnnotation::Array(elem_type)) => {
                // Array indexing
                self.constraints
                    .push((index_type.clone(), BuiltinTypes::number()));
                Ok(Type::Concrete(*elem_type.clone()))
            }
            Type::Concrete(TypeAnnotation::Basic(name)) => {
                // Check if this is a registered record schema (e.g., "rows" returns "row")
                if self.env.lookup_record_schema(name).is_some() {
                    self.constraints
                        .push((index_type.clone(), BuiltinTypes::number()));
                    Ok(Type::Concrete(TypeAnnotation::Basic(name.clone())))
                } else {
                    // For unknown types, create a constraint
                    let result_type = Type::Variable(TypeVar::fresh());
                    let var = TypeVar::fresh();

                    self.constraints.push((
                        object_type.clone(),
                        Type::Constrained {
                            var,
                            constraint: Box::new(TypeConstraint::Iterable),
                        },
                    ));

                    Ok(result_type)
                }
            }
            _ => {
                // For unknown types, create a constraint
                let result_type = Type::Variable(TypeVar::fresh());
                let var = TypeVar::fresh();

                self.constraints.push((
                    object_type.clone(),
                    Type::Constrained {
                        var,
                        constraint: Box::new(TypeConstraint::Iterable),
                    },
                ));

                Ok(result_type)
            }
        }
    }

    /// Infer type of function call
    pub(crate) fn infer_function_call(
        &mut self,
        name: &str,
        args: &[Expr],
        call_span: Span,
    ) -> TypeResult<Type> {
        // Infer argument types
        let arg_types: Vec<_> = args
            .iter()
            .map(|arg| self.infer_expr(arg))
            .collect::<Result<_, _>>()?;

        // Builtin arity special-cases that cannot be represented by a single
        // fixed-arity function type in the symbol table.
        if name == "print" {
            if arg_types.is_empty() {
                return Err(TypeError::ConstraintViolation(
                    "Function 'print' expects at least 1 argument, got 0".to_string(),
                ));
            }
            return Ok(BuiltinTypes::void());
        }

        if name == "range" {
            let actual_arity = arg_types.len();
            if !(1..=3).contains(&actual_arity) {
                return Err(TypeError::ConstraintViolation(format!(
                    "Function 'range' expects between 1 and 3 arguments, got {}",
                    actual_arity
                )));
            }
            let origin = self
                .lookup_callable_origin_for_name(name)
                .unwrap_or(call_span);
            for arg_ty in &arg_types {
                self.push_constraint_with_origin(arg_ty.clone(), BuiltinTypes::integer(), origin);
            }
            return Ok(BuiltinTypes::array(BuiltinTypes::integer()));
        }

        // Look up function type after argument inference so argument errors
        // (e.g. unknown property access) surface even when callee is undefined.
        let func_scheme = self
            .env
            .lookup(name)
            .ok_or_else(|| TypeError::UndefinedFunction(name.to_string()))?;

        // Instantiate with bounds to emit ImplementsTrait constraints for trait-bounded generics
        let (func_type, bound_constraints, default_substitutions) =
            func_scheme.instantiate_with_bounds();
        self.constraints.extend(bound_constraints);
        self.record_function_callsite(name, &arg_types);

        let origin = self
            .lookup_callable_origin_for_name(name)
            .unwrap_or(call_span);

        // Unknown callee types (e.g. unannotated higher-order params) are
        // constrained to callable shapes from this call site.
        if !matches!(
            &func_type,
            Type::Function { .. } | Type::Concrete(TypeAnnotation::Function { .. })
        ) {
            if matches!(
                &func_type,
                Type::Variable(_) | Type::Constrained { .. }
            ) {
                let result_type = Type::Variable(TypeVar::fresh());
                let expected_func_type =
                    BuiltinTypes::function(arg_types.clone(), result_type.clone());
                self.push_constraint_with_origin(func_type, expected_func_type, origin);
                return Ok(result_type);
            }
            return Err(TypeError::ConstraintViolation(format!(
                "'{}' is not callable",
                name
            )));
        }
        let (params, returns) = match &func_type {
            Type::Function { params, returns } => (params.clone(), returns.as_ref().clone()),
            Type::Concrete(TypeAnnotation::Function {
                params: concrete_params,
                returns: concrete_returns,
            }) => {
                let params: Vec<Type> = concrete_params
                    .iter()
                    .map(|p| Type::Concrete(p.type_annotation.clone()))
                    .collect();
                let returns = Type::Concrete(*concrete_returns.clone());
                (params, returns)
            }
            _ => unreachable!("non-function callees are handled above"),
        };

        let total_arity = params.len();
        let default_flags = self
            .callable_param_defaults
            .get(name)
            .cloned()
            .unwrap_or_else(|| vec![false; total_arity]);
        let required_arity = default_flags
            .iter()
            .position(|has_default| *has_default)
            .unwrap_or(total_arity);
        let actual_arity = arg_types.len();

        if actual_arity < required_arity || actual_arity > total_arity {
            return Err(TypeError::ConstraintViolation(format!(
                "Function '{}' expects between {} and {} arguments, got {}",
                name, required_arity, total_arity, actual_arity
            )));
        }

        let mut substitutions: std::collections::HashMap<TypeVar, Type> =
            std::collections::HashMap::new();
        for (param_ty, arg_ty) in params.iter().zip(arg_types.iter()) {
            Self::collect_call_substitutions(param_ty, arg_ty, &mut substitutions);
        }

        // Generic defaults apply only for unresolved type variables.
        for (var, default_type) in default_substitutions {
            if substitutions.contains_key(&var) {
                continue;
            }
            substitutions.insert(var.clone(), default_type.clone());
            self.push_constraint_with_origin(Type::Variable(var), default_type, origin);
        }

        let inferred_result_type = Self::apply_substitutions_to_type(&returns, &substitutions);

        let mut expected_param_types = arg_types.clone();
        for param_ty in params.iter().skip(actual_arity) {
            expected_param_types.push(Self::apply_substitutions_to_type(param_ty, &substitutions));
        }
        let expected_func_type =
            BuiltinTypes::function(expected_param_types, inferred_result_type.clone());
        self.push_constraint_with_origin(func_type, expected_func_type, origin);

        Ok(inferred_result_type)
    }

    /// Infer element type from iterator type
    pub(crate) fn infer_iterator_element_type(&self, iter_type: &Type) -> TypeResult<Type> {
        match iter_type {
            Type::Concrete(TypeAnnotation::Array(elem_type)) => {
                Ok(Type::Concrete(*elem_type.clone()))
            }
            // Iterating Table<T> produces Row<T>
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if name == "Table" && args.len() == 1 =>
            {
                Ok(Type::Concrete(TypeAnnotation::Generic {
                    name: "Row".to_string(),
                    args: args.clone(),
                }))
            }
            Type::Concrete(TypeAnnotation::Basic(name)) => {
                // Special case: "rows" iterates to produce "row" elements
                if name == "rows" {
                    Ok(BuiltinTypes::row())
                } else if self.env.lookup_record_schema(name).is_some() {
                    // If this is a registered schema type, it likely iterates to itself
                    Ok(Type::Concrete(TypeAnnotation::Basic(name.clone())))
                } else {
                    // For unknown iterators, return a fresh type variable
                    Ok(Type::Variable(TypeVar::fresh()))
                }
            }
            _ => {
                // For unknown iterators, return a fresh type variable
                Ok(Type::Variable(TypeVar::fresh()))
            }
        }
    }

    fn collect_call_substitutions(
        expected: &Type,
        actual: &Type,
        substitutions: &mut std::collections::HashMap<TypeVar, Type>,
    ) {
        match expected {
            Type::Variable(var) => {
                substitutions
                    .entry(var.clone())
                    .or_insert_with(|| actual.clone());
            }
            Type::Constrained { var, .. } => {
                substitutions
                    .entry(var.clone())
                    .or_insert_with(|| actual.clone());
            }
            Type::Generic {
                base: expected_base,
                args: expected_args,
            } => {
                if let Type::Generic {
                    base: actual_base,
                    args: actual_args,
                } = actual
                {
                    Self::collect_call_substitutions(expected_base, actual_base, substitutions);
                    for (exp_arg, act_arg) in expected_args.iter().zip(actual_args.iter()) {
                        Self::collect_call_substitutions(exp_arg, act_arg, substitutions);
                    }
                }
            }
            Type::Function {
                params: expected_params,
                returns: expected_returns,
            } => {
                if let Type::Function {
                    params: actual_params,
                    returns: actual_returns,
                } = actual
                {
                    for (exp_param, act_param) in expected_params.iter().zip(actual_params.iter()) {
                        Self::collect_call_substitutions(exp_param, act_param, substitutions);
                    }
                    Self::collect_call_substitutions(
                        expected_returns,
                        actual_returns,
                        substitutions,
                    );
                }
            }
            Type::Concrete(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine() -> TypeInferenceEngine {
        TypeInferenceEngine::new()
    }

    fn table_type(inner: &str) -> Type {
        Type::Concrete(TypeAnnotation::Generic {
            name: "Table".to_string(),
            args: vec![TypeAnnotation::Basic(inner.to_string())],
        })
    }

    fn row_type(inner: &str) -> Type {
        Type::Concrete(TypeAnnotation::Generic {
            name: "Row".to_string(),
            args: vec![TypeAnnotation::Basic(inner.to_string())],
        })
    }

    #[test]
    fn test_table_iteration_produces_row() {
        let engine = make_engine();
        let table = table_type("Candle");
        let element = engine.infer_iterator_element_type(&table).unwrap();

        // Iterating Table<Candle> should produce Row<Candle>
        match element {
            Type::Concrete(TypeAnnotation::Generic { name, args }) => {
                assert_eq!(name, "Row");
                assert_eq!(args.len(), 1);
                assert!(matches!(&args[0], TypeAnnotation::Basic(n) if n == "Candle"));
            }
            other => panic!("expected Row<Candle>, got {:?}", other),
        }
    }

    #[test]
    fn test_row_index_access_rejected() {
        let mut engine = make_engine();
        let row = row_type("Candle");
        let index = BuiltinTypes::string();

        // Dynamic index access on Row<T> should produce a type error
        let result = engine.infer_index_access(&row, &index);
        assert!(result.is_err());
    }

    #[test]
    fn test_row_property_access_falls_through() {
        let mut engine = make_engine();
        let row = row_type("Candle");

        // Property access on Row<T> should attempt to resolve on T
        // Since "Candle" isn't registered as a record schema, it falls through
        // to constraint-based inference (returns a fresh type variable)
        let result = engine.infer_property_access(&row, "open");
        assert!(result.is_ok());
    }

    #[test]
    fn test_intersection_property_access_resolves_member_field() {
        let mut engine = make_engine();
        let ty = Type::Concrete(TypeAnnotation::Intersection(vec![
            TypeAnnotation::Object(vec![shape_ast::ast::ObjectTypeField {
                name: "x".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            }]),
            TypeAnnotation::Object(vec![shape_ast::ast::ObjectTypeField {
                name: "z".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            }]),
        ]));

        let result = engine.infer_property_access(&ty, "z");
        assert!(result.is_ok(), "intersection member field should resolve");
    }
}
