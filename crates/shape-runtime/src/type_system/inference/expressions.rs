//! Expression-level type inference
//!
//! Handles type inference for all expression types.

use super::TypeInferenceEngine;
use crate::type_system::exhaustiveness;
use crate::type_system::*;
use shape_ast::ast::{Expr, Literal, Span, TypeAnnotation};
use shape_ast::interpolation::{
    InterpolationPart, parse_content_interpolation_with_mode, parse_interpolation_with_mode,
};

impl TypeInferenceEngine {
    /// Infer type of an expression
    pub fn infer_expr(&mut self, expr: &Expr) -> TypeResult<Type> {
        match expr {
            Expr::Literal(Literal::FormattedString { value, mode }, span) => {
                self.infer_formatted_string_interpolations(value, *mode, *span)?;
                Ok(BuiltinTypes::string())
            }

            Expr::Literal(Literal::ContentString { value, mode }, span) => {
                self.infer_content_string_interpolations(value, *mode, *span)?;
                Ok(Type::Concrete(TypeAnnotation::Basic("content".into())))
            }

            Expr::Literal(lit, _) => self.infer_literal(lit),

            Expr::Identifier(name, span) => self
                .env
                .lookup(name)
                .map(|scheme| scheme.instantiate())
                .or_else(|| {
                    // Fall back to a type reference for known struct type names.
                    // This enables static-path expressions like `Currency.symbol`
                    // where `Currency` is a type name, not a variable.
                    if self.struct_type_defs.contains_key(name.as_str())
                        || self.env.lookup_type_alias(name).is_some()
                    {
                        Some(Type::Concrete(TypeAnnotation::Reference(name.clone())))
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    // Recognize built-in namespace identifiers that have static
                    // constructor methods (e.g. DateTime.now(), Content.chart()).
                    match name.as_str() {
                        "DateTime" | "Content" => {
                            Some(Type::Concrete(TypeAnnotation::Reference(name.clone())))
                        }
                        _ => None,
                    }
                })
                .ok_or_else(|| {
                    self.register_undefined_variable_origin(name, *span);
                    TypeError::UndefinedVariable(name.clone())
                }),

            Expr::BinaryOp {
                left,
                op,
                right,
                span,
            } => {
                let left_type = self.infer_expr(left)?;
                let right_type = self.infer_expr(right)?;

                self.infer_binary_op(&left_type, op, &right_type, *span)
            }

            Expr::UnaryOp { op, operand, .. } => {
                let operand_type = self.infer_expr(operand)?;
                self.infer_unary_op(op, &operand_type)
            }

            Expr::PropertyAccess {
                object,
                property,
                span,
                ..
            } => {
                // Track the variable name for hoisting lookup
                let var_name = if let Expr::Identifier(name, _) = object.as_ref() {
                    Some(name.clone())
                } else {
                    None
                };

                // Set current access variable so hoisting can be looked up
                self.env.set_current_access_variable(var_name);

                let object_type = self.infer_expr(object)?;
                let result = self.infer_property_access(&object_type, property);
                if let Err(TypeError::UnknownProperty(_, missing)) = &result {
                    self.register_unknown_property_origin(missing, *span);
                }

                // Clear the current access variable
                self.env.set_current_access_variable(None);

                result
            }

            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                let object_type = self.infer_expr(object)?;
                let index_type = self.infer_expr(index)?;

                if let Some(end) = end_index {
                    let _end_type = self.infer_expr(end)?;
                    // For range indexing, return the same array type
                    Ok(object_type)
                } else {
                    self.infer_index_access(&object_type, &index_type)
                }
            }

            Expr::FunctionCall {
                name, args, span, ..
            } => self.infer_function_call(name, args, *span),

            Expr::EnumConstructor { enum_name, .. } => {
                Ok(Type::Concrete(TypeAnnotation::Reference(enum_name.clone())))
            }

            Expr::Array(elements, _) => {
                if elements.is_empty() {
                    // Empty array, create a fresh type variable
                    let elem_type = Type::Variable(TypeVar::fresh());
                    Ok(BuiltinTypes::array(elem_type))
                } else {
                    // Infer element type from first element
                    let first_type = self.infer_expr(&elements[0])?;

                    // All elements should have the same type
                    for elem in &elements[1..] {
                        let elem_type = self.infer_expr(elem)?;
                        self.constraints.push((first_type.clone(), elem_type));
                    }

                    Ok(BuiltinTypes::array(first_type))
                }
            }

            Expr::Object(entries, _) => {
                use shape_ast::ast::ObjectEntry;
                let mut field_types = Vec::new();

                for entry in entries {
                    match entry {
                        ObjectEntry::Field {
                            key,
                            value,
                            type_annotation,
                        } => {
                            let value_type = self.infer_expr(value)?;
                            let field_annotation = if let Some(ta) = type_annotation {
                                let annotated_type = Type::Concrete(ta.clone());
                                self.constraints.push((value_type.clone(), annotated_type));
                                ta.clone()
                            } else {
                                value_type.to_annotation().unwrap_or(TypeAnnotation::Any)
                            };
                            field_types.push(shape_ast::ast::ObjectTypeField {
                                name: key.clone(),
                                optional: false,
                                type_annotation: field_annotation,
                                annotations: vec![],
                            });
                        }
                        ObjectEntry::Spread(spread_expr) => {
                            // For spread, we infer the type and merge fields if it's an object
                            let spread_type = self.infer_expr(spread_expr)?;
                            match &spread_type {
                                Type::Concrete(TypeAnnotation::Object(spread_fields)) => {
                                    field_types.extend(spread_fields.clone());
                                }
                                Type::Concrete(TypeAnnotation::Reference(name)) => {
                                    // Named type (e.g., Point) — look up struct fields
                                    if let Some(struct_def) =
                                        self.struct_type_defs.get(name.as_str()).cloned()
                                    {
                                        for field in &struct_def.fields {
                                            field_types.push(
                                                shape_ast::ast::ObjectTypeField {
                                                    name: field.name.clone(),
                                                    optional: false,
                                                    type_annotation: field
                                                        .type_annotation
                                                        .clone(),
                                                    annotations: vec![],
                                                },
                                            );
                                        }
                                    }
                                }
                                _ => {
                                    // Fields from spread will be determined at runtime
                                }
                            }
                        }
                    }
                }

                Ok(Type::Concrete(TypeAnnotation::Object(field_types)))
            }

            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                let cond_type = self.infer_expr(condition)?;
                self.constraints.push((cond_type, BuiltinTypes::boolean()));

                let then_type = self.infer_expr(then_expr)?;

                if let Some(else_expr) = else_expr {
                    let else_type = self.infer_expr(else_expr)?;
                    // Both branches should have the same type
                    self.constraints.push((then_type.clone(), else_type));
                }

                Ok(then_type)
            }

            Expr::TypeAssertion {
                expr,
                type_annotation,
                ..
            } => {
                let expr_type = self.infer_expr(expr)?;
                if let TypeAnnotation::Optional(inner) = type_annotation {
                    // `as Type?` is the typed fallible-conversion form.
                    // It compiles to `Result<Type, AnyError>` and is validated
                    // statically against source/target strong types.
                    let target_type = self.resolve_type_annotation(inner.as_ref());
                    self.validate_fallible_conversion(&expr_type, &target_type)?;
                    return Ok(self.wrap_result_type(target_type));
                }

                let asserted_type = self.resolve_type_annotation(type_annotation);

                // Plain `as Type` is trait-dispatched conversion when Type is a
                // concrete named target supported by Into<Target>.
                if self.try_into_selector(&asserted_type).is_some() {
                    self.validate_infallible_conversion(&expr_type, &asserted_type)?;
                    return Ok(asserted_type);
                }

                // Plain `as Type` remains a strict assertion.
                self.constraints.push((expr_type, asserted_type.clone()));

                Ok(asserted_type)
            }

            Expr::InstanceOf {
                expr,
                type_annotation: _,
                ..
            } => {
                self.infer_expr(expr)?;
                Ok(BuiltinTypes::boolean())
            }

            // Method call: receiver.method(args)
            Expr::MethodCall {
                receiver,
                method,
                args,
                span: _,
                ..
            } => {
                let receiver_type = self.infer_expr(receiver)?;
                let arg_types: Vec<_> = args
                    .iter()
                    .map(|arg| self.infer_expr(arg))
                    .collect::<Result<_, _>>()?;

                // Try to resolve the method statically using the method table
                if let Some(result_type) =
                    self.method_table
                        .resolve_method_call(&receiver_type, method, &arg_types)
                {
                    return Ok(result_type);
                }

                // Fallback: treat receiver.method(...) as a callable field access
                // when the receiver is concretely object-like.
                //
                // For unresolved generic/constrained receivers (e.g. T: Displayable),
                // forcing a HasField constraint here over-constrains the receiver to a
                // structural object shape and breaks trait-bound method dispatch.
                let can_try_callable_field = !matches!(
                    &receiver_type,
                    Type::Variable(_)
                        | Type::Constrained { .. }
                        | Type::Concrete(TypeAnnotation::Any)
                );
                if can_try_callable_field {
                    if let Ok(field_type) = self.infer_property_access(&receiver_type, method) {
                        match field_type {
                            Type::Concrete(TypeAnnotation::Function { params, returns }) => {
                                let required_count = params.iter().filter(|p| !p.optional).count();
                                if arg_types.len() < required_count
                                    || arg_types.len() > params.len()
                                {
                                    return Err(TypeError::ArityMismatch(
                                        required_count,
                                        arg_types.len(),
                                    ));
                                }

                                for (arg_ty, param) in arg_types.iter().zip(params.iter()) {
                                    self.constraints.push((
                                        arg_ty.clone(),
                                        Type::Concrete(param.type_annotation.clone()),
                                    ));
                                }

                                return Ok(Type::Concrete(*returns));
                            }
                            Type::Function { params, returns } => {
                                if params.len() != arg_types.len() {
                                    return Err(TypeError::ArityMismatch(
                                        params.len(),
                                        arg_types.len(),
                                    ));
                                }

                                for (arg_ty, param_ty) in arg_types.iter().zip(params.iter()) {
                                    self.constraints.push((arg_ty.clone(), param_ty.clone()));
                                }

                                return Ok(*returns);
                            }
                            _ => {}
                        }
                    }
                }

                // Method not found in table - create a fresh type variable
                // This allows code to compile while deferring to runtime resolution
                // for user-defined methods or extension methods
                let result_type = Type::Variable(TypeVar::fresh());

                // Create a constraint that receiver must have this method
                self.constraints.push((
                    Type::Constrained {
                        var: TypeVar::fresh(),
                        constraint: Box::new(TypeConstraint::HasMethod {
                            method_name: method.clone(),
                            arg_types: arg_types.clone(),
                            return_type: Box::new(result_type.clone()),
                        }),
                    },
                    receiver_type,
                ));

                Ok(result_type)
            }

            // Match expression
            Expr::Match(match_expr, span) => {
                let scrutinee_type = self.infer_expr(&match_expr.scrutinee)?;

                // Collect all arm return types
                let mut arm_types: Vec<Type> = Vec::new();

                for arm in &match_expr.arms {
                    self.env.push_scope();

                    // Bind pattern variables
                    self.bind_pattern_vars(&arm.pattern)?;

                    // Check guard if present
                    if let Some(guard) = &arm.guard {
                        let guard_type = self.infer_expr(guard)?;
                        self.constraints.push((guard_type, BuiltinTypes::boolean()));
                    }

                    let body_type = self.infer_expr(&arm.body)?;
                    arm_types.push(body_type);

                    self.env.pop_scope();
                }

                // Check exhaustiveness for closed types (enums, unions).
                // Union-typed scrutinees use typed-pattern coverage based on concrete variants.
                let result = if matches!(
                    scrutinee_type.to_annotation(),
                    Some(TypeAnnotation::Union(_))
                ) {
                    exhaustiveness::check_exhaustiveness_for_type(match_expr, &scrutinee_type)
                } else if let Some(semantic_type) = scrutinee_type.to_semantic() {
                    let resolved_type = self.resolve_named_to_enum(&semantic_type);
                    match resolved_type {
                        crate::type_system::semantic::SemanticType::Enum { .. } => {
                            exhaustiveness::check_exhaustiveness(match_expr, &resolved_type)
                        }
                        _ => exhaustiveness::check_exhaustiveness_for_type(
                            match_expr,
                            &scrutinee_type,
                        ),
                    }
                } else {
                    exhaustiveness::check_exhaustiveness_for_type(match_expr, &scrutinee_type)
                };
                if let Some(error) = result.to_error() {
                    if let TypeError::NonExhaustiveMatch { enum_name, .. } = &error {
                        self.register_non_exhaustive_match_origin(enum_name, *span);
                    }
                    return Err(error);
                }

                // Determine result type: unify if same, create nominal union if different
                let result_type = if arm_types.is_empty() {
                    Type::Variable(TypeVar::fresh())
                } else if self.all_types_equal(&arm_types) {
                    // All arms have the same type - use that type
                    arm_types[0].clone()
                } else {
                    // Heterogeneous arms - create NOMINAL union type with auto-generated brand
                    self.create_nominal_union(&arm_types)?
                };

                Ok(result_type)
            }

            // If expression
            Expr::If(if_expr, _) => {
                let cond_type = self.infer_expr(&if_expr.condition)?;
                self.constraints.push((cond_type, BuiltinTypes::boolean()));

                let then_type = self.infer_expr(&if_expr.then_branch)?;

                if let Some(else_branch) = &if_expr.else_branch {
                    let else_type = self.infer_expr(else_branch)?;
                    self.constraints.push((then_type.clone(), else_type));
                }

                Ok(then_type)
            }

            // While expression
            Expr::While(while_expr, _) => {
                let cond_type = self.infer_expr(&while_expr.condition)?;
                self.constraints.push((cond_type, BuiltinTypes::boolean()));

                self.infer_expr(&while_expr.body)?;
                // While loops return void (or the break value if any)
                Ok(BuiltinTypes::void())
            }

            // For expression
            Expr::For(for_expr, _) => {
                self.env.push_scope();

                let iter_type = self.infer_expr(&for_expr.iterable)?;
                let element_type = self.infer_iterator_element_type(&iter_type)?;

                // Bind pattern variable
                if let Some(name) = for_expr.pattern.as_simple_name() {
                    self.env.define(name, TypeScheme::mono(element_type));
                }

                self.infer_expr(&for_expr.body)?;
                self.env.pop_scope();

                // For expressions return void (or collected values if used as expression)
                Ok(BuiltinTypes::void())
            }

            // Loop expression
            Expr::Loop(loop_expr, _) => {
                self.infer_expr(&loop_expr.body)?;
                // Loop returns void (or the break value)
                Ok(BuiltinTypes::void())
            }

            // Let expression
            Expr::Let(let_expr, _) => {
                self.env.push_scope();

                let var_type = if let Some(ann) = &let_expr.type_annotation {
                    self.resolve_type_annotation(ann)
                } else {
                    Type::Variable(TypeVar::fresh())
                };

                if let Some(value) = &let_expr.value {
                    let value_type = self.infer_expr(value)?;
                    self.constraints.push((var_type.clone(), value_type));
                }

                if let Some(name) = let_expr.pattern.as_simple_name() {
                    self.env.define(name, TypeScheme::mono(var_type));
                }

                let body_type = self.infer_expr(&let_expr.body)?;
                self.env.pop_scope();

                Ok(body_type)
            }

            // Assignment expression
            Expr::Assign(assign_expr, _) => {
                let value_type = self.infer_expr(&assign_expr.value)?;
                let target_type = if let Expr::PropertyAccess {
                    object, property, ..
                } = assign_expr.target.as_ref()
                {
                    if let Expr::Identifier(var_name, _) = object.as_ref() {
                        // Mark first so `a.y = ...` can resolve a hoisted field target even
                        // before it has been read once.
                        self.env.mark_hoisted_field_initialized(var_name, property);

                        self.env.set_current_access_variable(Some(var_name.clone()));
                        let target = self.infer_expr(object).and_then(|object_type| {
                            self.infer_property_assignment_target(&object_type, property)
                        });
                        self.env.set_current_access_variable(None);
                        target?
                    } else {
                        self.infer_expr(&assign_expr.target)?
                    }
                } else {
                    self.infer_expr(&assign_expr.target)?
                };

                // Assignment must be type-compatible with the target field/variable.
                self.constraints
                    .push((target_type.clone(), value_type.clone()));

                // Record field evolution for property assignments (a.x = v)
                if let Expr::PropertyAccess {
                    object, property, ..
                } = assign_expr.target.as_ref()
                {
                    if let Expr::Identifier(var_name, _) = object.as_ref() {
                        // Keep the variable's object shape in sync for later expressions.
                        self.env
                            .upsert_object_field(var_name, property, value_type.clone());

                        // Convert inference type to semantic type for evolution tracking
                        if let Some(semantic_type) = value_type.to_semantic() {
                            // Ignore errors - evolution tracking is best-effort
                            let _ =
                                self.env
                                    .record_field_assignment(var_name, property, semantic_type);
                        }
                    }
                }

                // Assignment returns the value type
                Ok(value_type)
            }

            // Block expression
            Expr::Block(block, _) => {
                self.env.push_scope();
                let mut last_type = BuiltinTypes::void();

                for item in &block.items {
                    last_type = match item {
                        shape_ast::ast::BlockItem::VariableDecl(decl) => {
                            self.infer_variable_decl(decl)?;
                            BuiltinTypes::void()
                        }
                        shape_ast::ast::BlockItem::Assignment(_assign) => BuiltinTypes::void(),
                        shape_ast::ast::BlockItem::Statement(_stmt) => BuiltinTypes::void(),
                        shape_ast::ast::BlockItem::Expression(expr) => self.infer_expr(expr)?,
                    };
                }

                self.env.pop_scope();
                Ok(last_type)
            }

            // Function expression
            Expr::FunctionExpr {
                params,
                return_type,
                body,
                ..
            } => {
                self.env.push_scope();
                self.push_fallible_scope();

                let mut param_types = Vec::new();
                for param in params {
                    let param_type = if let Some(ann) = &param.type_annotation {
                        Type::Concrete(ann.clone())
                    } else {
                        Type::Variable(TypeVar::fresh())
                    };
                    param_types.push(param_type.clone());
                    // Define all identifiers from the pattern
                    for name in param.get_identifiers() {
                        self.env.define(&name, TypeScheme::mono(param_type.clone()));
                    }
                }

                let local_constraint_start = self.constraints.len();
                let inferred_result = self.infer_callable_return_type(body, return_type.is_some());
                self.refine_callable_param_types_from_local_constraints(
                    &mut param_types,
                    &self.constraints[local_constraint_start..],
                    true,
                );
                let is_fallible = self.pop_fallible_scope();
                self.env.pop_scope();
                let inferred_return = inferred_result?;

                let ret_type = if let Some(ann) = return_type {
                    let annotated = Type::Concrete(ann.clone());
                    self.constraints.push((inferred_return, annotated.clone()));
                    annotated
                } else {
                    inferred_return
                };
                let ret_type = self.apply_fallibility_to_return_type(ret_type, is_fallible);

                Ok(BuiltinTypes::function(param_types, ret_type))
            }

            // List comprehension
            Expr::ListComprehension(comp, _) => {
                self.env.push_scope();

                // Process each clause (for x in items, for y in other_items, etc.)
                for clause in &comp.clauses {
                    let iter_type = self.infer_expr(&clause.iterable)?;
                    let element_type = self.infer_iterator_element_type(&iter_type)?;

                    if let Some(name) = clause.pattern.as_identifier() {
                        self.env.define(name, TypeScheme::mono(element_type));
                    }

                    if let Some(filter) = &clause.filter {
                        let cond_type = self.infer_expr(filter)?;
                        self.constraints.push((cond_type, BuiltinTypes::boolean()));
                    }
                }

                let elem_type = self.infer_expr(&comp.element)?;
                self.env.pop_scope();

                Ok(BuiltinTypes::array(elem_type))
            }

            // Data references - return generic object type
            Expr::DataRef(_, _) | Expr::DataDateTimeRef(_, _) => {
                Ok(Type::Concrete(TypeAnnotation::Basic("object".to_string())))
            }

            // Data relative access
            Expr::DataRelativeAccess { .. } => {
                Ok(Type::Concrete(TypeAnnotation::Basic("object".to_string())))
            }

            // Time references
            Expr::TimeRef(_, _) | Expr::DateTime(_, _) => Ok(Type::Concrete(
                TypeAnnotation::Basic("datetime".to_string()),
            )),

            // Duration
            Expr::Duration(_, _) => Ok(Type::Concrete(TypeAnnotation::Basic(
                "duration".to_string(),
            ))),

            // Pattern reference
            Expr::PatternRef(_, _) => Ok(BuiltinTypes::pattern()),

            // Spread expression
            Expr::Spread(inner, _) => self.infer_expr(inner),

            // Range expression
            Expr::Range { start, end, .. } => {
                let element_type = if let Some(s) = start {
                    let start_type = self.infer_expr(s)?;
                    if let Some(e) = end {
                        let end_type = self.infer_expr(e)?;
                        self.constraints.push((start_type.clone(), end_type));
                    }
                    start_type
                } else if let Some(e) = end {
                    self.infer_expr(e)?
                } else {
                    Type::Concrete(TypeAnnotation::Any)
                };
                Ok(Type::Concrete(TypeAnnotation::Generic {
                    name: "Range".to_string(),
                    args: vec![element_type.to_annotation().unwrap_or(TypeAnnotation::Any)],
                }))
            }

            // Timeframe context
            Expr::TimeframeContext { expr, span: _, .. } => self.infer_expr(expr),

            // Control flow - these return void or break/continue semantics
            Expr::Break(value, _) => {
                if let Some(val) = value {
                    self.infer_expr(val)
                } else {
                    Ok(BuiltinTypes::void())
                }
            }

            Expr::Continue(_) => Ok(BuiltinTypes::void()),

            Expr::Return(value, _) => {
                let return_type = if let Some(val) = value {
                    self.infer_expr(val)?
                } else {
                    BuiltinTypes::void()
                };
                self.record_return_type(return_type.clone());
                Ok(return_type)
            }

            // Unit
            Expr::Unit(_) => Ok(BuiltinTypes::void()),

            // Try operator for Result/Option propagation
            // The ? operator:
            // 1. Supports Result<T> and Option<T> / T? values
            // 2. Extracts and returns the inner success type
            // 3. Marks the containing function as fallible (contagious Result)
            Expr::TryOperator(inner, _) => {
                let inner_type = self.infer_expr(inner)?;

                // Mark the current function scope as fallible
                self.mark_current_scope_fallible();

                if let Some(unwrapped) = self.try_unwrap_inner_type(&inner_type) {
                    return Ok(unwrapped);
                }

                // When the inner type is an unresolved type variable (e.g. untyped
                // lambda parameter), we cannot reject it — it may later resolve to
                // Result<T,E> or Option<T>.  Return a fresh type variable for the
                // unwrapped value and let downstream constraints refine it.
                if self.type_contains_unresolved_vars(&inner_type) {
                    return Ok(Type::Variable(TypeVar::fresh()));
                }

                Err(TypeError::ConstraintViolation(format!(
                    "try operator '?' expects Result<T, E> or Option<T>, found '{}'",
                    self.render_type_for_diag(&inner_type)
                )))
            }

            // Named impl selector does not change the value type.
            // Trait-specific validation happens in call sites (e.g. formatting).
            Expr::UsingImpl { expr, .. } => self.infer_expr(expr),

            // Simulation call with inline parameters
            Expr::SimulationCall {
                name: _, params, ..
            } => {
                // Infer types for all parameter expressions
                for (_, value_expr) in params {
                    self.infer_expr(value_expr)?;
                }
                // Return a fresh type variable - actual type depends on runtime
                Ok(Type::Variable(TypeVar::fresh()))
            }

            // Window expressions return numbers
            Expr::WindowExpr(_, _) => Ok(BuiltinTypes::number()),

            // Fuzzy comparisons return boolean
            Expr::FuzzyComparison { left, right, .. } => {
                self.infer_expr(left)?;
                self.infer_expr(right)?;
                Ok(BuiltinTypes::boolean())
            }

            // FromQuery should be desugared before type inference
            // If we see one, treat it as returning Array of the select type
            Expr::FromQuery(from_query, _) => {
                // Infer source type (should be an array)
                let _source_ty = self.infer_expr(&from_query.source)?;
                // Infer clause expressions
                for clause in &from_query.clauses {
                    match clause {
                        shape_ast::QueryClause::Where(pred) => {
                            self.infer_expr(pred)?;
                        }
                        shape_ast::QueryClause::OrderBy(specs) => {
                            for spec in specs {
                                self.infer_expr(&spec.key)?;
                            }
                        }
                        shape_ast::QueryClause::GroupBy { element, key, .. } => {
                            self.infer_expr(element)?;
                            self.infer_expr(key)?;
                        }
                        shape_ast::QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            self.infer_expr(source)?;
                            self.infer_expr(left_key)?;
                            self.infer_expr(right_key)?;
                        }
                        shape_ast::QueryClause::Let { value, .. } => {
                            self.infer_expr(value)?;
                        }
                    }
                }
                let select_ty = self.infer_expr(&from_query.select)?;
                Ok(BuiltinTypes::array(select_ty))
            }
            Expr::StructLiteral {
                type_name, fields, ..
            } => self.infer_struct_literal_type(type_name, fields),

            // Await expression - infer the type of the inner expression
            Expr::Await(inner, _) => self.infer_expr(inner),

            // Join expression - infer types of all branches
            Expr::Join(join_expr, _) => {
                for branch in &join_expr.branches {
                    self.infer_expr(&branch.expr)?;
                }
                Ok(Type::Variable(TypeVar::fresh()))
            }

            // Annotated expression - infer the type of the target
            Expr::Annotated { target, .. } => self.infer_expr(target),

            // Async let - spawns a task, the expression type is a future handle
            Expr::AsyncLet(async_let, _) => {
                self.infer_expr(&async_let.expr)?;
                Ok(Type::Variable(TypeVar::fresh()))
            }

            // Async scope - cancellation boundary, type is the body's type
            Expr::AsyncScope(inner, _) => self.infer_expr(inner),

            // Comptime block - evaluated at compile time, returns Any for now
            Expr::Comptime(_, _) => Ok(Type::Variable(TypeVar::fresh())),

            // Comptime for - unrolled at compile time, returns Unit
            Expr::ComptimeFor(_, _) => Ok(Type::Concrete(TypeAnnotation::Void)),

            // Reference expression - infer the inner expression type
            Expr::Reference { expr: inner, .. } => self.infer_expr(inner),
        }
    }

    fn infer_struct_literal_type(
        &mut self,
        type_name: &str,
        fields: &[(String, Expr)],
    ) -> TypeResult<Type> {
        use std::collections::HashMap;

        let mut inferred_field_types: HashMap<String, Type> = HashMap::new();
        for (field_name, value_expr) in fields {
            let field_type = self.infer_expr(value_expr)?;
            inferred_field_types.insert(field_name.clone(), field_type);
        }

        let Some(struct_def) = self.struct_type_defs.get(type_name).cloned() else {
            return Ok(Type::Concrete(TypeAnnotation::Reference(
                type_name.to_string(),
            )));
        };

        let type_params = struct_def.type_params.unwrap_or_default();
        if type_params.is_empty() {
            return Ok(Type::Concrete(TypeAnnotation::Reference(
                type_name.to_string(),
            )));
        }

        let mut param_bindings: HashMap<String, Vec<Type>> = HashMap::new();
        for field in struct_def.fields.iter().filter(|f| !f.is_comptime) {
            let Some(actual_field_type) = inferred_field_types.get(&field.name) else {
                continue;
            };
            self.bind_type_params_from_annotation(
                &field.type_annotation,
                actual_field_type,
                &type_params,
                &mut param_bindings,
            );
        }

        let mut resolved_args: Vec<Type> = Vec::with_capacity(type_params.len());
        for tp in &type_params {
            let candidates = param_bindings.remove(&tp.name).unwrap_or_default();
            let resolved = self.resolve_struct_type_param_arg(tp, candidates)?;
            resolved_args.push(resolved);
        }

        let all_default = type_params
            .iter()
            .zip(resolved_args.iter())
            .all(|(tp, arg)| {
                self.default_type_for_type_param(tp)
                    .map_or(false, |default_type| {
                        if self.types_equal(&default_type, arg) {
                            return true;
                        }
                        matches!(
                            (&default_type, arg),
                            (
                                Type::Concrete(TypeAnnotation::Reference(a)),
                                Type::Concrete(TypeAnnotation::Basic(b)),
                            ) | (
                                Type::Concrete(TypeAnnotation::Basic(a)),
                                Type::Concrete(TypeAnnotation::Reference(b)),
                            ) if a == b
                        )
                    })
            });

        if all_default {
            Ok(Type::Concrete(TypeAnnotation::Reference(
                type_name.to_string(),
            )))
        } else {
            Ok(Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                    type_name.to_string(),
                ))),
                args: resolved_args,
            })
        }
    }

    fn bind_type_params_from_annotation(
        &mut self,
        annotation: &TypeAnnotation,
        actual: &Type,
        type_params: &[shape_ast::ast::TypeParam],
        bindings: &mut std::collections::HashMap<String, Vec<Type>>,
    ) {
        let is_type_param = |name: &str| type_params.iter().any(|tp| tp.name == name);

        match annotation {
            TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name)
                if is_type_param(name) =>
            {
                let entry = bindings.entry(name.clone()).or_default();
                if !entry
                    .iter()
                    .any(|existing| self.types_equal(existing, actual))
                {
                    entry.push(actual.clone());
                }
            }
            TypeAnnotation::Array(inner) => {
                if let Type::Concrete(TypeAnnotation::Array(actual_inner)) = actual {
                    self.bind_type_params_from_annotation(
                        inner,
                        &Type::Concrete((**actual_inner).clone()),
                        type_params,
                        bindings,
                    );
                }
            }
            TypeAnnotation::Optional(inner) => {
                if let Type::Concrete(TypeAnnotation::Optional(actual_inner)) = actual {
                    self.bind_type_params_from_annotation(
                        inner,
                        &Type::Concrete((**actual_inner).clone()),
                        type_params,
                        bindings,
                    );
                }
            }
            TypeAnnotation::Generic { name, args } => {
                if let Type::Generic {
                    base,
                    args: actual_args,
                } = actual
                {
                    let base_name = match base.as_ref() {
                        Type::Concrete(TypeAnnotation::Reference(n))
                        | Type::Concrete(TypeAnnotation::Basic(n)) => Some(n.as_str()),
                        _ => None,
                    };
                    if base_name == Some(name.as_str()) {
                        for (expected_arg, actual_arg) in args.iter().zip(actual_args.iter()) {
                            self.bind_type_params_from_annotation(
                                expected_arg,
                                actual_arg,
                                type_params,
                                bindings,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn resolve_struct_type_param_arg(
        &mut self,
        tp: &shape_ast::ast::TypeParam,
        candidates: Vec<Type>,
    ) -> TypeResult<Type> {
        if candidates.is_empty() {
            if let Some(default_type) = self.default_type_for_type_param(tp) {
                return Ok(default_type);
            }
            return Err(TypeError::GenericTypeError {
                message: format!(
                    "Could not infer type argument '{}' for generic struct",
                    tp.name
                ),
                symbol: None,
            });
        }

        if candidates.len() == 1 {
            return Ok(candidates.into_iter().next().unwrap());
        }

        self.combine_return_types(&candidates)
    }

    fn default_type_for_type_param(&self, tp: &shape_ast::ast::TypeParam) -> Option<Type> {
        if let Some(default_ann) = &tp.default_type {
            return Some(Type::Concrete(default_ann.clone()));
        }
        None
    }

    fn infer_formatted_string_interpolations(
        &mut self,
        value: &str,
        mode: shape_ast::ast::InterpolationMode,
        span: Span,
    ) -> TypeResult<()> {
        let parts = parse_interpolation_with_mode(value, mode)
            .map_err(|err| TypeError::ConstraintViolation(err.to_string()))?;

        for part in parts {
            if let InterpolationPart::Expression { expr, .. } = part {
                let parsed_expr = shape_ast::parser::parse_expression_str(&expr)
                    .map_err(|err| TypeError::ConstraintViolation(err.to_string()))?;
                match self.infer_expr(&parsed_expr) {
                    Ok(_) => {}
                    Err(TypeError::UnknownProperty(type_name, property)) => {
                        self.overwrite_unknown_property_origin(&property, span);
                        return Err(TypeError::UnknownProperty(type_name, property));
                    }
                    Err(err) => return Err(err),
                }
            }
        }

        Ok(())
    }

    /// Infer types of interpolation expressions within a content string literal (c"...").
    ///
    /// Uses `parse_content_interpolation_with_mode` which understands content-specific
    /// format directives like `fg(red)`, `bold`, `border(rounded)` in addition to
    /// the standard `fixed(N)` numeric format.
    fn infer_content_string_interpolations(
        &mut self,
        value: &str,
        mode: shape_ast::ast::InterpolationMode,
        span: Span,
    ) -> TypeResult<()> {
        let parts = parse_content_interpolation_with_mode(value, mode)
            .map_err(|err| TypeError::ConstraintViolation(err.to_string()))?;

        for part in parts {
            if let InterpolationPart::Expression { expr, .. } = part {
                let parsed_expr = shape_ast::parser::parse_expression_str(&expr)
                    .map_err(|err| TypeError::ConstraintViolation(err.to_string()))?;
                match self.infer_expr(&parsed_expr) {
                    Ok(_) => {}
                    Err(TypeError::UnknownProperty(type_name, property)) => {
                        self.overwrite_unknown_property_origin(&property, span);
                        return Err(TypeError::UnknownProperty(type_name, property));
                    }
                    Err(err) => return Err(err),
                }
            }
        }

        Ok(())
    }

    /// Bind pattern variables to the type environment
    ///
    /// This recursively processes all patterns, creating fresh type variables
    /// for each bound identifier. Handles:
    /// - Simple identifiers: `x` binds x to a fresh type var
    /// - Array patterns: `[a, b]` binds a and b
    /// - Object patterns: `{x, y}` binds x and y
    /// - Constructor patterns: `Some(x)` binds x
    /// - Wildcards: `_` binds nothing
    pub(crate) fn bind_pattern_vars(
        &mut self,
        pattern: &shape_ast::ast::Pattern,
    ) -> TypeResult<()> {
        use shape_ast::ast::{Pattern, PatternConstructorFields};

        match pattern {
            Pattern::Identifier(name) => {
                let var_type = Type::Variable(TypeVar::fresh());
                self.env.define(name, TypeScheme::mono(var_type));
            }
            Pattern::Typed {
                name,
                type_annotation,
            } => {
                let var_type = self.resolve_type_annotation(type_annotation);
                self.env.define(name, TypeScheme::mono(var_type));
            }
            Pattern::Literal(_) => {
                // Literals don't bind variables
            }
            Pattern::Wildcard => {
                // Wildcards don't bind variables
            }
            Pattern::Array(patterns) => {
                for p in patterns {
                    self.bind_pattern_vars(p)?;
                }
            }
            Pattern::Object(fields) => {
                for (_, p) in fields {
                    self.bind_pattern_vars(p)?;
                }
            }
            Pattern::Constructor { fields, .. } => {
                match fields {
                    PatternConstructorFields::Unit => {
                        // No variables to bind
                    }
                    PatternConstructorFields::Tuple(patterns) => {
                        for p in patterns {
                            self.bind_pattern_vars(p)?;
                        }
                    }
                    PatternConstructorFields::Struct(fields) => {
                        for (_, p) in fields {
                            self.bind_pattern_vars(p)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Best-effort static unwrapping for `expr?`.
    fn try_unwrap_inner_type(&self, ty: &Type) -> Option<Type> {
        match ty {
            Type::Generic { base, args } if !args.is_empty() => match base.as_ref() {
                Type::Concrete(TypeAnnotation::Reference(name))
                | Type::Concrete(TypeAnnotation::Basic(name))
                    if name == "Result" || name == "Option" =>
                {
                    Some(args[0].clone())
                }
                _ => None,
            },
            Type::Concrete(TypeAnnotation::Optional(inner)) => {
                Some(Type::Concrete(inner.as_ref().clone()))
            }
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if (name == "Result" || name == "Option") && !args.is_empty() =>
            {
                Some(Type::Concrete(args[0].clone()))
            }
            _ => None,
        }
    }

    fn validate_fallible_conversion(&self, source: &Type, target: &Type) -> TypeResult<()> {
        if self.type_contains_unresolved_vars(source) || self.type_contains_unresolved_vars(target)
        {
            return Ok(());
        }

        if self.types_equal(source, target) {
            return Ok(());
        }

        let source_name = self.try_into_type_name(source).ok_or_else(|| {
            TypeError::InvalidAssertion(
                self.render_type_for_diag(source),
                format!("{}?", self.render_type_for_diag(target)),
            )
        })?;
        let target_selector = self.try_into_selector(target).ok_or_else(|| {
            TypeError::InvalidAssertion(
                self.render_type_for_diag(source),
                format!("{}?", self.render_type_for_diag(target)),
            )
        })?;

        if self.has_try_into_impl(&source_name, &target_selector) {
            return Ok(());
        }

        Err(TypeError::InvalidAssertion(
            self.render_type_for_diag(source),
            format!("{}?", self.render_type_for_diag(target)),
        ))
    }

    fn validate_infallible_conversion(&self, source: &Type, target: &Type) -> TypeResult<()> {
        if self.type_contains_unresolved_vars(source) || self.type_contains_unresolved_vars(target)
        {
            return Ok(());
        }

        if self.types_equal(source, target) {
            return Ok(());
        }

        let source_name = self.try_into_type_name(source).ok_or_else(|| {
            TypeError::InvalidAssertion(
                self.render_type_for_diag(source),
                self.render_type_for_diag(target),
            )
        })?;
        let target_selector = self.try_into_selector(target).ok_or_else(|| {
            TypeError::InvalidAssertion(
                self.render_type_for_diag(source),
                self.render_type_for_diag(target),
            )
        })?;

        if self.has_into_impl(&source_name, &target_selector) {
            return Ok(());
        }

        Err(TypeError::InvalidAssertion(
            self.render_type_for_diag(source),
            self.render_type_for_diag(target),
        ))
    }

    fn render_type_for_diag(&self, ty: &Type) -> String {
        if matches!(ty, Type::Variable(_) | Type::Constrained { .. }) {
            return "unknown".to_string();
        }
        ty.to_annotation()
            .map(|ann| match ann {
                TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => name,
                other => format!("{other:?}"),
            })
            .unwrap_or_else(|| format!("{ty:?}"))
    }

    fn has_try_into_impl(&self, source_type: &str, target_selector: &str) -> bool {
        self.env
            .lookup_trait_impl_named("TryInto", source_type, target_selector)
            .is_some()
            || self.env.lookup_trait_impl("TryInto", source_type).is_some()
    }

    fn has_into_impl(&self, source_type: &str, target_selector: &str) -> bool {
        self.env
            .lookup_trait_impl_named("Into", source_type, target_selector)
            .is_some()
            || self.env.lookup_trait_impl("Into", source_type).is_some()
    }

    fn try_into_type_name(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Concrete(TypeAnnotation::Basic(name))
            | Type::Concrete(TypeAnnotation::Reference(name))
            | Type::Concrete(TypeAnnotation::Generic { name, .. }) => {
                Some(Self::canonical_try_into_name(name))
            }
            Type::Generic { base, .. } => match base.as_ref() {
                Type::Concrete(TypeAnnotation::Basic(name))
                | Type::Concrete(TypeAnnotation::Reference(name))
                | Type::Concrete(TypeAnnotation::Generic { name, .. }) => {
                    Some(Self::canonical_try_into_name(name))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn try_into_selector(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Concrete(TypeAnnotation::Basic(name))
            | Type::Concrete(TypeAnnotation::Reference(name))
            | Type::Concrete(TypeAnnotation::Generic { name, .. }) => {
                Some(Self::canonical_try_into_name(name))
            }
            Type::Generic { base, .. } => match base.as_ref() {
                Type::Concrete(TypeAnnotation::Basic(name))
                | Type::Concrete(TypeAnnotation::Reference(name))
                | Type::Concrete(TypeAnnotation::Generic { name, .. }) => {
                    Some(Self::canonical_try_into_name(name))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn canonical_try_into_name(name: &str) -> String {
        match name {
            "boolean" | "Boolean" | "Bool" => "bool".to_string(),
            "String" => "string".to_string(),
            "Number" => "number".to_string(),
            "Int" => "int".to_string(),
            "Decimal" => "decimal".to_string(),
            _ => name.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::Span;

    fn test_span() -> Span {
        Span { start: 0, end: 0 }
    }

    #[test]
    fn try_operator_unwraps_result_and_marks_scope_fallible() {
        let mut engine = TypeInferenceEngine::new();
        engine.push_fallible_scope();

        let result_number = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Result".to_string(),
            ))),
            args: vec![BuiltinTypes::number()],
        };
        engine.env.define("value", TypeScheme::mono(result_number));

        let expr = Expr::TryOperator(
            Box::new(Expr::Identifier("value".to_string(), test_span())),
            test_span(),
        );

        let inferred = engine.infer_expr(&expr).expect("result? should infer");
        assert_eq!(inferred, BuiltinTypes::number());
        assert!(engine.pop_fallible_scope());
    }

    #[test]
    fn try_operator_unwraps_optional_type_and_marks_scope_fallible() {
        let mut engine = TypeInferenceEngine::new();
        engine.push_fallible_scope();

        let optional_number = Type::Concrete(TypeAnnotation::Optional(Box::new(
            TypeAnnotation::Basic("number".to_string()),
        )));
        engine
            .env
            .define("value", TypeScheme::mono(optional_number));

        let expr = Expr::TryOperator(
            Box::new(Expr::Identifier("value".to_string(), test_span())),
            test_span(),
        );

        let inferred = engine.infer_expr(&expr).expect("option? should infer");
        assert_eq!(inferred, BuiltinTypes::number());
        assert!(engine.pop_fallible_scope());
    }

    #[test]
    fn try_operator_unwraps_ok_constructor_call() {
        let mut engine = TypeInferenceEngine::new();
        engine.push_fallible_scope();

        let expr =
            shape_ast::parser::parse_expression_str("Ok(1)?").expect("expression should parse");
        let inferred = engine.infer_expr(&expr).expect("Ok(1)? should infer");
        assert_eq!(inferred, BuiltinTypes::integer());
        assert!(engine.pop_fallible_scope());
    }

    #[test]
    fn try_operator_rejects_non_fallible_operand() {
        let mut engine = TypeInferenceEngine::new();
        engine.push_fallible_scope();
        let expr = shape_ast::parser::parse_expression_str("42?").expect("expression should parse");
        let err = engine
            .infer_expr(&expr)
            .expect_err("plain value ? should be rejected");
        assert!(
            matches!(err, TypeError::ConstraintViolation(_)),
            "expected ConstraintViolation, got {:?}",
            err
        );
    }

    #[test]
    fn fallible_type_assertion_as_optional_returns_typed_result() {
        let mut engine = TypeInferenceEngine::new();
        let _ = engine.env.register_trait_impl_named(
            "TryInto",
            "string",
            "int",
            vec!["tryInto".to_string()],
        );
        let expr = shape_ast::parser::parse_expression_str("\"42\" as int?")
            .expect("fallible cast expression should parse");

        let inferred = engine
            .infer_expr(&expr)
            .expect("fallible cast should infer");

        match inferred {
            Type::Generic { base, args } => {
                assert!(matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name)) if name == "Result"
                ));
                assert_eq!(args.len(), 2);
                assert_eq!(args[0], BuiltinTypes::integer());
                assert_eq!(
                    args[1],
                    Type::Concrete(TypeAnnotation::Reference("AnyError".to_string()))
                );
            }
            other => panic!("expected Result<int, AnyError>, got {:?}", other),
        }
    }

    #[test]
    fn infallible_type_assertion_as_uses_into_impl() {
        let mut engine = TypeInferenceEngine::new();
        let _ =
            engine
                .env
                .register_trait_impl_named("Into", "string", "int", vec!["into".to_string()]);
        let expr = shape_ast::parser::parse_expression_str("\"42\" as int")
            .expect("cast expression should parse");

        let inferred = engine
            .infer_expr(&expr)
            .expect("into-backed cast should infer");
        assert_eq!(inferred, BuiltinTypes::integer());
    }

    #[test]
    fn infallible_type_assertion_rejects_unsupported_static_conversion() {
        let mut engine = TypeInferenceEngine::new();
        let expr =
            shape_ast::parser::parse_expression_str("{ x: 1 } as int").expect("expression parse");
        let err = engine
            .infer_expr(&expr)
            .expect_err("object -> int cast should fail without Into impl");
        assert!(
            matches!(err, TypeError::InvalidAssertion(_, _)),
            "expected InvalidAssertion, got {:?}",
            err
        );
    }

    #[test]
    fn fallible_type_assertion_accepts_named_try_into_impl() {
        let mut engine = TypeInferenceEngine::new();
        let _ = engine.env.register_trait_impl_named(
            "TryInto",
            "Price",
            "int",
            vec!["tryInto".to_string()],
        );
        engine.env.define(
            "value",
            TypeScheme::mono(Type::Concrete(TypeAnnotation::Reference(
                "Price".to_string(),
            ))),
        );

        let expr =
            shape_ast::parser::parse_expression_str("value as int?").expect("expression parses");
        let inferred = engine
            .infer_expr(&expr)
            .expect("named TryInto impl should satisfy static validation");

        match inferred {
            Type::Generic { base, args } => {
                assert!(matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name)) if name == "Result"
                ));
                assert_eq!(args.len(), 2);
                assert_eq!(args[0], BuiltinTypes::integer());
            }
            other => panic!("expected Result<int, AnyError>, got {:?}", other),
        }
    }

    #[test]
    fn fallible_type_assertion_rejects_unsupported_static_conversion() {
        let mut engine = TypeInferenceEngine::new();
        let expr = shape_ast::parser::parse_expression_str("{ x: 1 } as int?")
            .expect("expression should parse");
        let err = engine
            .infer_expr(&expr)
            .expect_err("object -> int fallible cast should fail statically");
        assert!(
            matches!(err, TypeError::InvalidAssertion(_, _)),
            "expected InvalidAssertion, got {:?}",
            err
        );
    }

    #[test]
    fn fallible_type_assertion_in_program_uses_preceding_try_into_impl() {
        let mut engine = TypeInferenceEngine::new();
        let program = shape_ast::parser::parse_program(
            r#"
impl TryInto<int> for string as int {
  method tryInto() {
    __try_into_int(self)
  }
}

fn parse(raw: string) -> Result<int> {
  let n = (raw as int?)?
  Ok(n)
}
"#,
        )
        .expect("program should parse");

        let types = engine
            .infer_program(&program)
            .expect("program-level inference should see prior TryInto impl");

        assert!(
            types.contains_key("parse"),
            "expected inferred function type"
        );
    }

    #[test]
    fn fallible_type_assertion_in_program_with_callsite_uses_preceding_try_into_impl() {
        let mut engine = TypeInferenceEngine::new();
        let program = shape_ast::parser::parse_program(
            r#"
impl TryInto<int> for string as int {
  method tryInto() {
    __try_into_int(self)
  }
}

fn parse(raw: string) -> Result<int> {
  let n = (raw as int?)?
  Ok(n)
}

match parse("not-int") {
  Ok(v) => v
  Err(_) => -1
}
"#,
        )
        .expect("program should parse");

        let types = engine
            .infer_program(&program)
            .expect("program-level inference should keep TryInto impl with callsite");

        assert!(
            types.contains_key("parse"),
            "expected inferred function type"
        );
    }

    #[test]
    fn infallible_type_assertion_in_program_uses_preceding_into_impl() {
        let mut engine = TypeInferenceEngine::new();
        let program = shape_ast::parser::parse_program(
            r#"
impl Into<int> for string as int {
  method into() {
    __into_int(self)
  }
}

fn parse(raw: string) -> int {
  raw as int
}
"#,
        )
        .expect("program should parse");

        let types = engine
            .infer_program(&program)
            .expect("program-level inference should see prior Into impl");

        assert!(
            types.contains_key("parse"),
            "expected inferred function type"
        );
    }
}
