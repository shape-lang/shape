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
                        // A bare `Reference("Pair")` reaches here when an
                        // all-default generic literal (`Pair { first: 1,
                        // second: 2 }` where `type Pair<A=int,B=int>`)
                        // short-circuits to a bare reference in
                        // `infer_struct_literal_type`. The field's annotation
                        // is then the abstract param name (`A`/`B`); returning
                        // it raw makes `p.first + p.second` typecheck as
                        // `A + B`. If the annotation names a type param that
                        // carries a default, substitute that default so the
                        // field resolves to its defaulted concrete type. A
                        // param with no default (or a field that names no
                        // param) is unchanged — genuinely-abstract fields
                        // stay abstract, genuine UnknownProperty still errors.
                        if let Some(default) = Self::default_for_named_type_param(
                            &struct_def,
                            &field.type_annotation,
                        ) {
                            return Ok(default);
                        }
                        return Ok(Type::Concrete(field.type_annotation.clone()));
                    }
                }
                return Err(TypeError::UnknownProperty(
                    name.to_string(),
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

        // D-α.2: built-in `.length` property on length-bearing types
        // (Array<T>, String, HashMap<K,V>, Set<T>, Deque<T>,
        // PriorityQueue<T>, Range). The compiler emits typed length opcodes
        // for these (see `compiler/expressions/property_access.rs:288-335`,
        // method registry entries at `executor/objects/method_registry.rs`).
        // Without this arm the inference layer falls through to the
        // `HasField` fallback and the result variable stays `unknown`,
        // breaking expressions like `arr.length - 1` (KC #6(c)).
        if property == "length" && Self::is_length_bearing_type(object_type) {
            return Ok(BuiltinTypes::integer());
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
                if self.struct_type_defs.contains_key(name.as_str()) {
                    return Err(TypeError::UnknownProperty(
                        name.to_string(),
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
                    // A field may carry a `tyvar` marker (an object literal
                    // built from an unannotated parameter). Decode it back to
                    // a `Type::Variable` so a binop / comparison on the field
                    // resolves through the unifier once callsite propagation
                    // has bound the parameter.
                    if let Some(var) = annotation_as_tyvar(&field.type_annotation) {
                        return Ok(self.unifier.apply_substitutions(&Type::Variable(var)));
                    }
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
                let result_type = self.fresh_type_var();
                let var = self.fresh_var();

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
            Type::Concrete(ann) => ann.as_type_name_str(),
            _ => None,
        }
    }

    /// Returns true when `object_type` is one of the built-in length-bearing
    /// types whose `.length` property the compiler lowers to a typed length
    /// opcode (`ArrayLenTyped`, `MapLenTyped`, `StringLenTyped`, etc.). Used
    /// by `infer_property_access_internal` to short-circuit the
    /// `HasField` fallback so binary ops on the result are typed `int`.
    ///
    /// D-α.2. Mirrors the receiver-classification done by
    /// `try_resolve_typed_length_local` in
    /// `crates/shape-vm/src/compiler/expressions/property_access.rs` and the
    /// `"length"` PHF entries in
    /// `crates/shape-vm/src/executor/objects/method_registry.rs`.
    fn is_length_bearing_type(object_type: &Type) -> bool {
        let name = match object_type {
            Type::Concrete(TypeAnnotation::Array(_)) => return true,
            Type::Concrete(TypeAnnotation::Basic(n)) => n.as_str(),
            Type::Concrete(TypeAnnotation::Generic { name, .. }) => name.as_str(),
            Type::Generic { base, .. } => match Self::generic_base_name(base) {
                Some(n) => n,
                None => return false,
            },
            _ => return false,
        };
        matches!(
            name,
            "string"
                | "String"
                | "Vec"
                | "Array"
                | "HashMap"
                | "Set"
                | "Deque"
                | "PriorityQueue"
                | "Range"
        )
    }

    /// If `field_annotation` names one of `struct_def`'s declared type params,
    /// and that param has a default type, return the defaulted type. Used by
    /// the bare-`Reference` struct field-access arm so an all-default generic
    /// literal (`Pair { first: 1 }` for `type Pair<A = int>`) resolves field
    /// access to the param's default rather than the abstract param name.
    ///
    /// Returns `None` when the annotation is not a bare type-name, names no
    /// type param, or names a param without a default — in those cases the
    /// caller keeps the field's original (possibly abstract) annotation.
    fn default_for_named_type_param(
        struct_def: &shape_ast::ast::StructTypeDef,
        field_annotation: &TypeAnnotation,
    ) -> Option<Type> {
        // Only a bare name can alias a type param (e.g. `A`, not `Array<A>`).
        let field_name = match field_annotation {
            TypeAnnotation::Basic(_) | TypeAnnotation::Reference(_) => {
                field_annotation.as_type_name_str()?
            }
            _ => return None,
        };
        let type_params = struct_def.type_params.as_ref()?;
        let tp = type_params.iter().find(|tp| tp.name() == field_name)?;
        let default_ann = tp.default_type()?;
        Some(Type::Concrete(default_ann.clone()))
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
                // TODO(B.3): const generic params alias into this map under
                // their name; B.3 will route const args through a separate
                // value-level substitution pass.
                bindings.insert(tp.name().to_string(), arg_ann);
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
            ann @ (TypeAnnotation::Basic(_) | TypeAnnotation::Reference(_)) => {
                let name = ann.as_type_name_str().unwrap();
                bindings
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| annotation.clone())
            }
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
        let result_type = self.fresh_type_var();
        let var = self.fresh_var();

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

    /// Push an element-carrying `Indexable` constraint for an `obj[i]`
    /// access whose object type is not yet a concrete array.
    ///
    /// Returns the fresh element-type variable. The constraint records that
    /// variable inside `TypeConstraint::Indexable`, so when the object's
    /// type later resolves to a concrete `Array<T>` (e.g. via callsite
    /// unification of an unannotated parameter), `ConstraintSolver::
    /// apply_bounds` binds the element variable to `T`. Without the carried
    /// element type the access would return a disconnected fresh variable
    /// and a downstream `obj[i] + ...` would see `unknown` operands.
    fn push_indexable_constraint(&mut self, object_type: &Type) -> Type {
        let result_type = self.fresh_type_var();
        let var = self.fresh_var();
        self.constraints.push((
            object_type.clone(),
            Type::Constrained {
                var,
                constraint: Box::new(TypeConstraint::Indexable(Box::new(
                    result_type.clone(),
                ))),
            },
        ));
        result_type
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
                    // For unknown types, create an element-carrying index
                    // constraint so the element type connects to the
                    // object's resolved type (mirrors `HasField`).
                    Ok(self.push_indexable_constraint(object_type))
                }
            }
            _ => {
                // For unknown types, create an element-carrying index
                // constraint (mirrors `HasField`) — see above.
                Ok(self.push_indexable_constraint(object_type))
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
            func_scheme.instantiate_with_bounds(&mut self.type_var_gen);
        self.constraints.extend(bound_constraints);
        self.record_function_callsite(name, &arg_types);

        // v0.3.3 c4-4D — HOF callee-param-inference propagation.
        //
        // When a named function is passed as a value argument to another
        // function (`apply(double, 21)`), the standard callsite-union scheme
        // records `arg_types` against the OUTER callee (`apply`) only —
        // `record_function_callsite("apply", [Fn(...), int])`. The INNER
        // callee (`double`) gets no direct call site, so
        // `callsite_param_types["double"]` stays empty. The
        // `refine_numeric_params_post_callsite` last-resort default then
        // collapses double's parameter to `number`, and the bytecode
        // compiler emits `MulNumber` on call-supplied `int` bits, producing
        // a Float64 denormal at runtime (audit-04 §8 c4-4D root).
        //
        // The body of the outer callee constrains how its function-typed
        // parameter is called against its other parameters: the body of
        // `fn apply(f, x) { f(x) }` pushes the constraint
        // `Type::Variable(f_src) ~ Function { params: [Variable(x_src)], returns: _ }`
        // before the outer call site is processed (each function's body is
        // inferred eagerly in `infer_item`). Reading that constraint here
        // lets us derive a synthetic callsite for the inner callee:
        // "double's parameter[0] gets called with whatever the OUTER
        // callsite passed as the corresponding parameter of `apply`".
        //
        // Specifically: when args[i] is `Expr::Identifier(callee_name)` for
        // a known named function and `callable_param_source_vars[name][i]`
        // is bound to a `Function { params: [Variable(p_var)], returns: _ }`
        // shape via constraints, find which of `name`'s OTHER params has
        // source var `p_var` — call its index `j`. Then record a synthetic
        // callsite for `callee_name` with the arg type at position `j`.
        //
        // Only fires for HOF args that are bare named-function identifiers
        // (`Expr::Identifier`). Closures and lambdas use the closure-eager
        // refinement at `refine_callable_param_types_from_local_constraints`
        // (is_closure=true), which already collapses their numeric params
        // to `number` at body-inference time; they are out of scope here.
        //
        // Soundness: only the source vars are read, only existing constraints
        // are inspected (no new ones pushed), and the synthetic callsite
        // recording goes through the same `record_function_callsite` path
        // that direct call sites use — `refine_numeric_params_post_callsite`
        // then resolves the inner callee's parameter from the recorded
        // concrete type. No type kind is fabricated.
        self.propagate_hof_arg_callsites(name, args, &arg_types);

        let origin = self
            .lookup_callable_origin_for_name(name)
            .unwrap_or(call_span);

        // Unknown callee types (e.g. unannotated higher-order params) are
        // constrained to callable shapes from this call site.
        if !matches!(
            &func_type,
            Type::Function { .. } | Type::Concrete(TypeAnnotation::Function { .. })
        ) {
            if matches!(&func_type, Type::Variable(_) | Type::Constrained { .. }) {
                let result_type = self.fresh_type_var();
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
    pub(crate) fn infer_iterator_element_type(&mut self, iter_type: &Type) -> TypeResult<Type> {
        match iter_type {
            Type::Concrete(TypeAnnotation::Array(elem_type)) => {
                Ok(Type::Concrete(*elem_type.clone()))
            }
            // Iterating Table<T> produces Row<T>
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if name == "Table" && args.len() == 1 =>
            {
                Ok(Type::Concrete(TypeAnnotation::Generic {
                    name: "Row".into(),
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
                    Ok(self.fresh_type_var())
                }
            }
            _ => {
                // For unknown iterators, return a fresh type variable
                Ok(self.fresh_type_var())
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

    /// v0.3.3 c4-4D: HOF callee-param-inference propagation.
    ///
    /// At the call site of an outer function (`apply(double, 21)`), find
    /// each argument that is a bare named-function identifier and propagate
    /// a synthetic callsite for that inner callee using the outer body's
    /// call-shape constraint.
    ///
    /// See the comment block at the call site in `infer_function_call` for
    /// the full rationale + soundness argument. The mechanism in three
    /// steps:
    ///
    ///  1. Locate `args[i]` of the form `Expr::Identifier(callee_name)`
    ///     where `callee_name` resolves to a known named function (i.e.
    ///     has its own callable_param_source_vars entry).
    ///  2. Look up the outer function's i-th parameter source-var. Scan
    ///     `self.constraints` for `Variable(outer_param_i_src) ~
    ///     Function { params: [...], returns: _ }` (the body-imposed
    ///     call-shape constraint pushed by `infer_function_call` when the
    ///     outer body invokes its own parameter).
    ///  3. For each parameter slot in that call-shape, if the slot is
    ///     itself a bare `Variable(outer_param_j_src)`, find which of the
    ///     outer's other parameters has source var `outer_param_j_src` —
    ///     index `j`. Use `arg_types[j]` as the synthetic callsite arg for
    ///     `callee_name` at the same parameter position.
    ///
    /// Stays in inference-engine state: no new constraints are pushed; no
    /// type kind is fabricated. The synthetic callsite goes through the
    /// existing `record_function_callsite` path, so downstream widening +
    /// `refine_numeric_params_post_callsite` resolve the inner callee's
    /// parameter from the recorded concrete type exactly as if a direct
    /// `double(21)` call site had existed.
    pub(crate) fn propagate_hof_arg_callsites(
        &mut self,
        outer_name: &str,
        args: &[Expr],
        arg_types: &[Type],
    ) {
        eprintln!("HOF: outer_name={} args={} arg_types={:?}", outer_name, args.len(), arg_types);
        // Read the outer function's parameter source-vars. An annotated
        // outer parameter has `None` here; we need an unannotated
        // function-typed slot whose body imposed a call-shape constraint,
        // so `None` slots are skipped.
        let Some(outer_source_vars) = self.callable_param_source_vars.get(outer_name).cloned()
        else {
            return;
        };
        eprintln!("HOF: outer_source_vars={:?}", outer_source_vars);

        // Build outer_param_j_src → j lookup for resolving "this slot
        // references the outer's parameter j" once we read the call-shape
        // constraint.
        let outer_var_to_index: HashMap<TypeVar, usize> = outer_source_vars
            .iter()
            .enumerate()
            .filter_map(|(j, sv)| sv.as_ref().map(|v| (v.clone(), j)))
            .collect();

        for (i, arg) in args.iter().enumerate() {
            let Expr::Identifier(callee_name, _) = arg else {
                continue;
            };
            // The argument must reference a known named function whose
            // callsite-union scheme can consume the synthetic record.
            // (Closures/lambdas use the eager closure-collapse path at
            // body-inference time; they're out of scope.)
            if !self.callable_param_source_vars.contains_key(callee_name) {
                continue;
            }
            // The outer's i-th source-var must be the unannotated kind
            // whose body imposed a Fn-shape constraint.
            let Some(Some(outer_param_i_src)) = outer_source_vars.get(i) else {
                continue;
            };

            // Find a constraint `Variable(outer_param_i_src) ~
            // Function { params: [...], returns: _ }` in either direction.
            // Multiple such constraints can exist (e.g. if the outer body
            // calls `f` more than once); record from each.
            let outer_param_i_src = outer_param_i_src.clone();
            let snapshot: Vec<(Type, Type)> = self.constraints.clone();
            for (lhs, rhs) in &snapshot {
                let body_params = match (lhs, rhs) {
                    (Type::Variable(v), Type::Function { params, .. })
                    | (Type::Function { params, .. }, Type::Variable(v))
                        if v == &outer_param_i_src =>
                    {
                        params
                    }
                    _ => continue,
                };

                // Map each body-call parameter slot back to one of the
                // outer's parameter source vars (the call-shape `f(x)`
                // where `x` is outer param j tells us "callee receives
                // outer's param j"). Then take the synthetic arg type
                // from `arg_types[j]`.
                let mut synthetic_arg_types: Vec<Type> = Vec::with_capacity(body_params.len());
                let mut ok = true;
                for slot in body_params {
                    let Type::Variable(slot_var) = slot else {
                        ok = false;
                        break;
                    };
                    let Some(&j) = outer_var_to_index.get(slot_var) else {
                        ok = false;
                        break;
                    };
                    let Some(arg_ty) = arg_types.get(j) else {
                        ok = false;
                        break;
                    };
                    synthetic_arg_types.push(arg_ty.clone());
                }
                if !ok {
                    continue;
                }

                // The synthetic callsite goes through the normal record
                // path — `apply_callsite_unions` + `refine_numeric_params_post_callsite`
                // pick it up exactly as if `callee_name(arg_ty)` had been
                // written directly.
                eprintln!("HOF: synthetic callsite for {} arg_types={:?}", callee_name, synthetic_arg_types);
                self.record_function_callsite(callee_name, &synthetic_arg_types);
            }
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
            name: "Table".into(),
            args: vec![TypeAnnotation::Basic(inner.to_string())],
        })
    }

    fn row_type(inner: &str) -> Type {
        Type::Concrete(TypeAnnotation::Generic {
            name: "Row".into(),
            args: vec![TypeAnnotation::Basic(inner.to_string())],
        })
    }

    #[test]
    fn test_table_iteration_produces_row() {
        let mut engine = make_engine();
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
