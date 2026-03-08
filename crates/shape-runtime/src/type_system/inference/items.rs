//! Item-level type inference
//!
//! Handles type inference for top-level items: functions, patterns, variables, etc.

use super::TypeInferenceEngine;
use crate::type_system::*;
use shape_ast::ast::{
    DestructurePattern, ForeignFunctionDef, FunctionDef, InterfaceMember, Item, Statement,
    TraitMember, TypeAnnotation, TypeName, VarKind, VariableDecl,
};
use std::collections::HashMap;

impl TypeInferenceEngine {
    /// Predeclare symbols needed for order-independent inference.
    ///
    /// This mirrors the compiler's first-pass registration so functions and
    /// extend methods can be referenced before their textual declaration.
    pub(crate) fn predeclare_item(&mut self, item: &Item) -> TypeResult<()> {
        match item {
            Item::Function(func, _) => self.predeclare_function_signature(func),
            Item::ForeignFunction(def, _) => self.predeclare_foreign_function(def),
            Item::StructType(struct_def, _) => self.predeclare_struct_type(struct_def),
            Item::Export(export, _) => {
                if let shape_ast::ast::ExportItem::Function(func) = &export.item {
                    self.predeclare_function_signature(func)?;
                } else if let shape_ast::ast::ExportItem::ForeignFunction(def) = &export.item {
                    self.predeclare_foreign_function(def)?;
                } else if let shape_ast::ast::ExportItem::Struct(struct_def) = &export.item {
                    self.predeclare_struct_type(struct_def)?;
                }
                Ok(())
            }
            Item::Extend(extend, _) => self.register_extend(extend),
            _ => Ok(()),
        }
    }

    fn predeclare_function_signature(&mut self, func: &FunctionDef) -> TypeResult<()> {
        self.callable_param_defaults.insert(
            func.name.clone(),
            func.params
                .iter()
                .map(|p| p.default_value.is_some())
                .collect(),
        );

        let param_types: Vec<Type> = func
            .params
            .iter()
            .map(|p| {
                p.type_annotation
                    .as_ref()
                    .map(|ann| self.resolve_type_annotation(ann))
                    .unwrap_or_else(|| Type::Variable(TypeVar::fresh()))
            })
            .collect();

        let return_type = func
            .return_type
            .as_ref()
            .map(|ann| self.resolve_type_annotation(ann))
            .unwrap_or_else(|| Type::Variable(TypeVar::fresh()));

        let scheme =
            self.make_function_scheme(func, BuiltinTypes::function(param_types, return_type));
        self.env.define(&func.name, scheme);
        Ok(())
    }

    fn predeclare_foreign_function(&mut self, def: &ForeignFunctionDef) -> TypeResult<()> {
        let param_types: Vec<Type> = def
            .params
            .iter()
            .map(|p| {
                p.type_annotation
                    .as_ref()
                    .map(|ann| self.resolve_type_annotation(ann))
                    .unwrap_or_else(|| Type::Variable(TypeVar::fresh()))
            })
            .collect();

        let return_type = def
            .return_type
            .as_ref()
            .map(|ann| self.resolve_type_annotation(ann))
            .unwrap_or_else(|| Type::Variable(TypeVar::fresh()));

        let func_type = BuiltinTypes::function(param_types, return_type);
        let scheme = TypeScheme::mono(func_type);
        self.env.define(&def.name, scheme);
        Ok(())
    }

    fn predeclare_struct_type(
        &mut self,
        struct_def: &shape_ast::ast::StructTypeDef,
    ) -> TypeResult<()> {
        self.struct_type_defs
            .insert(struct_def.name.clone(), struct_def.clone());

        // Predeclare the nominal struct alias before callable signature
        // predeclaration so signatures like Vec<Measurement> in foreign
        // functions can resolve consistently to structural object shapes.
        let fields = struct_def
            .fields
            .iter()
            .filter(|f| !f.is_comptime)
            .map(|f| shape_ast::ast::ObjectTypeField {
                name: f.name.clone(),
                optional: f.default_value.is_some(),
                type_annotation: f.type_annotation.clone(),
                annotations: vec![],
            })
            .collect();
        self.env
            .define_type_alias(&struct_def.name, &TypeAnnotation::Object(fields), None);
        Ok(())
    }

    /// Infer types for a top-level item
    pub(crate) fn infer_item(
        &mut self,
        item: &Item,
        types: &mut HashMap<String, Type>,
    ) -> TypeResult<()> {
        match item {
            Item::Function(func, _) => {
                let func_type = self.infer_function(func)?;
                // Create polymorphic type scheme for generic functions
                let scheme = self.make_function_scheme(func, func_type.clone());
                self.env.define(&func.name, scheme);
                types.insert(func.name.clone(), func_type);
            }
            Item::ForeignFunction(_, _) => {
                // Foreign function bodies are opaque — type already predeclared
            }
            Item::VariableDecl(decl, _) => {
                let var_type = self.infer_variable_decl(decl)?;
                if let Some(name) = decl.pattern.as_identifier() {
                    types.insert(name.to_string(), var_type.clone());
                } else {
                    for name in decl.pattern.get_identifiers() {
                        let inferred = self
                            .env
                            .lookup(&name)
                            .map(|scheme| scheme.instantiate())
                            .unwrap_or_else(|| var_type.clone());
                        types.insert(name, inferred);
                    }
                }
            }
            Item::Statement(stmt, _) => {
                if let Statement::VariableDecl(decl, _) = stmt {
                    let var_type = self.infer_variable_decl(decl)?;
                    if let Some(name) = decl.pattern.as_identifier() {
                        types.insert(name.to_string(), var_type.clone());
                    } else {
                        for name in decl.pattern.get_identifiers() {
                            let inferred = self
                                .env
                                .lookup(&name)
                                .map(|scheme| scheme.instantiate())
                                .unwrap_or_else(|| var_type.clone());
                            types.insert(name, inferred);
                        }
                    }
                } else {
                    // Tolerate UndefinedFunction errors from expression statements
                    // (e.g. calling builtins like print that aren't registered in
                    // the type env) without killing the entire program's inference.
                    match self.infer_statement(stmt) {
                        Ok(_) | Err(TypeError::UndefinedFunction(_)) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
            Item::TypeAlias(alias, _) => {
                // Type aliases don't need inference, just register them with optional overrides
                self.env.define_type_alias(
                    &alias.name,
                    &alias.type_annotation,
                    alias.meta_param_overrides.clone(),
                );
            }
            Item::StructType(struct_def, _) => {
                self.struct_type_defs
                    .insert(struct_def.name.clone(), struct_def.clone());
                // Struct type definitions are registered as nominal aliases
                // to their structural object shape.
                let fields = struct_def
                    .fields
                    .iter()
                    .filter(|f| !f.is_comptime)
                    .map(|f| shape_ast::ast::ObjectTypeField {
                        name: f.name.clone(),
                        optional: f.default_value.is_some(),
                        type_annotation: f.type_annotation.clone(),
                        annotations: vec![],
                    })
                    .collect();
                self.env
                    .define_type_alias(&struct_def.name, &TypeAnnotation::Object(fields), None);
            }
            Item::Interface(interface, _) => {
                // Register interface in type environment
                self.env.define_interface(interface);
            }
            Item::Enum(enum_def, _) => {
                // Register enum for exhaustiveness checking
                self.env.register_enum(enum_def);
            }
            Item::Trait(trait_def, _) => {
                self.register_trait(trait_def)?;
            }
            Item::Impl(impl_block, _) => {
                self.register_impl(impl_block)?;
            }
            Item::Extend(extend, _) => {
                self.register_extend(extend)?;
            }
            Item::Export(export, _) => match &export.item {
                shape_ast::ast::ExportItem::Function(func) => {
                    let func_type = self.infer_function(func)?;
                    let scheme = self.make_function_scheme(func, func_type.clone());
                    self.env.define(&func.name, scheme);
                    types.insert(func.name.clone(), func_type);
                }
                shape_ast::ast::ExportItem::TypeAlias(alias) => {
                    self.env.define_type_alias(
                        &alias.name,
                        &alias.type_annotation,
                        alias.meta_param_overrides.clone(),
                    );
                }
                shape_ast::ast::ExportItem::Struct(struct_def) => {
                    self.struct_type_defs
                        .insert(struct_def.name.clone(), struct_def.clone());
                    let fields = struct_def
                        .fields
                        .iter()
                        .filter(|f| !f.is_comptime)
                        .map(|f| shape_ast::ast::ObjectTypeField {
                            name: f.name.clone(),
                            optional: f.default_value.is_some(),
                            type_annotation: f.type_annotation.clone(),
                            annotations: vec![],
                        })
                        .collect();
                    self.env.define_type_alias(
                        &struct_def.name,
                        &TypeAnnotation::Object(fields),
                        None,
                    );
                }
                shape_ast::ast::ExportItem::Trait(trait_def) => {
                    self.register_trait(trait_def)?;
                }
                _ => {}
            },
            _ => {} // Other items handled separately
        }

        Ok(())
    }

    /// Infer type of a function
    ///
    /// Implements contagious Result inference: if the function body contains
    /// any `?` operators, the return type is automatically wrapped in Result<T>.
    /// Also handles generic functions with type parameters.
    pub(crate) fn infer_function(&mut self, func: &FunctionDef) -> TypeResult<Type> {
        self.env.push_scope();
        self.push_fallible_scope();
        self.register_callable_origin_for_name(&func.name, func.name_span);

        // Positional defaults are only well-defined for trailing parameters.
        let mut saw_default = false;
        for param in &func.params {
            if param.default_value.is_some() {
                saw_default = true;
            } else if saw_default {
                self.env.pop_scope();
                self.pop_fallible_scope();
                return Err(TypeError::ConstraintViolation(
                    "Required parameter cannot follow a parameter with a default value".to_string(),
                ));
            }
        }

        // Create type variables for type parameters
        let mut type_param_vars = Vec::new();
        if let Some(type_params) = &func.type_params {
            for tp in type_params {
                let var = TypeVar::new(tp.name.clone());
                type_param_vars.push(var.clone());
                // Define type param in scope so it can be referenced in param/return types
                self.env
                    .define(&tp.name, TypeScheme::mono(Type::Variable(var)));
            }
        }

        // Collect parameter types
        let mut param_types = Vec::new();
        let mut unannotated_param_vars: Vec<TypeVar> = Vec::new();
        let mut param_source_vars: Vec<Option<TypeVar>> = Vec::new();

        for param in &func.params {
            let param_type = if let Some(ann) = &param.type_annotation {
                param_source_vars.push(None);
                self.resolve_type_annotation(ann)
            } else {
                let var = TypeVar::fresh();
                unannotated_param_vars.push(var.clone());
                param_source_vars.push(Some(var.clone()));
                Type::Variable(var)
            };

            param_types.push(param_type.clone());
            // Define all identifiers from the pattern
            for name in param.get_identifiers() {
                self.env.define(&name, TypeScheme::mono(param_type.clone()));
            }
        }
        self.callable_param_source_vars
            .insert(func.name.clone(), param_source_vars);
        self.callable_param_defaults.insert(
            func.name.clone(),
            func.params
                .iter()
                .map(|p| p.default_value.is_some())
                .collect(),
        );

        for (param, param_type) in func.params.iter().zip(param_types.iter()) {
            if let Some(default_expr) = &param.default_value {
                let default_type = self.infer_expr(default_expr)?;
                self.constraints.push((param_type.clone(), default_type));
            }
        }

        // Infer return type from annotation or create fresh variable
        let declared_return_type = if let Some(ann) = &func.return_type {
            self.resolve_type_annotation(ann)
        } else {
            Type::Variable(TypeVar::fresh())
        };

        // Infer callable return type from all explicit returns (or final expression)
        let local_constraint_start = self.constraints.len();
        let inferred_result =
            self.infer_callable_return_type(&func.body, func.return_type.is_some());
        self.refine_callable_param_types_from_local_constraints(
            &mut param_types,
            &self.constraints[local_constraint_start..],
            true,
        );
        let local_constraints = &self.constraints[local_constraint_start..];
        let local_origin = self
            .find_origin_for_callable_param_constraints(&unannotated_param_vars, local_constraints);

        // Check if function is fallible (contains ? operators)
        let is_fallible = self.pop_fallible_scope();
        self.env.pop_scope();
        let inferred_return_type = inferred_result?;

        if func.return_type.is_none() {
            let mut return_vars = std::collections::HashSet::new();
            self.collect_type_vars(&inferred_return_type, &mut return_vars);

            let mut allowed_vars: std::collections::HashSet<TypeVar> =
                type_param_vars.iter().cloned().collect();
            allowed_vars.extend(unannotated_param_vars.iter().cloned());

            if return_vars.iter().any(|var| !allowed_vars.contains(var))
                && matches!(inferred_return_type, Type::Generic { .. })
            {
                return Err(TypeError::GenericTypeError {
                    message: format!("Could not infer generic return type for '{}'", func.name),
                    symbol: Some(func.name.clone()),
                });
            }
        }

        // Determine the actual return type
        self.constraints
            .push((inferred_return_type.clone(), declared_return_type.clone()));

        // If deferred return-union members were recorded on the inferred return
        // variable, transfer them to the declared return variable that is
        // exposed in the final function type.
        if let (Type::Variable(inferred_var), Type::Variable(declared_var)) =
            (&inferred_return_type, &declared_return_type)
        {
            self.register_return_var_alias(declared_var.clone(), inferred_var.clone());
            // Only transfer deferred return-union members when an explicit
            // return annotation exists and therefore the declared return var is
            // the one exposed in the final function type.
            if func.return_type.is_some() && inferred_var != declared_var {
                if let Some(members) = self.pending_return_unions.remove(inferred_var) {
                    self.record_pending_return_union(declared_var.clone(), members);
                }
            }
        }

        // For unannotated functions, keep the inferred return shape as the
        // source of truth so fallibility wrapping does not produce
        // Result<Result<T>> when the body already returns a Result<T>.
        let return_base = if func.return_type.is_some() {
            declared_return_type
        } else {
            inferred_return_type
        };
        let actual_return_type = self.apply_fallibility_to_return_type(return_base, is_fallible);
        let function_type = BuiltinTypes::function(param_types, actual_return_type);
        if let Some(origin) = local_origin {
            self.register_callable_origin_for_name(&func.name, origin);
        }

        Ok(function_type)
    }

    /// Resolve a type annotation, converting type parameter references to type variables
    pub(crate) fn resolve_type_annotation(&self, ann: &TypeAnnotation) -> Type {
        match ann {
            // Check if this is a type parameter reference
            TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => {
                if let Some(scheme) = self.env.lookup(name) {
                    // If it's a type parameter (a type variable), use it
                    if let Type::Variable(_) = &scheme.ty {
                        return scheme.ty.clone();
                    }
                }
                if let Some(alias_entry) = self.env.lookup_type_alias(name) {
                    return self.resolve_type_annotation(&alias_entry.type_annotation);
                }
                Type::Concrete(ann.clone())
            }
            TypeAnnotation::Array(elem) => {
                let elem_type = self.resolve_type_annotation(elem);
                BuiltinTypes::array(elem_type)
            }
            TypeAnnotation::Tuple(elems) => {
                let resolved: Vec<TypeAnnotation> = elems
                    .iter()
                    .map(|e| {
                        self.resolve_type_annotation(e)
                            .to_annotation()
                            .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string()))
                    })
                    .collect();
                Type::Concrete(TypeAnnotation::Tuple(resolved))
            }
            TypeAnnotation::Object(fields) => {
                let resolved_fields = fields
                    .iter()
                    .map(|f| shape_ast::ast::ObjectTypeField {
                        name: f.name.clone(),
                        optional: f.optional,
                        type_annotation: self
                            .resolve_type_annotation(&f.type_annotation)
                            .to_annotation()
                            .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string())),
                        annotations: vec![],
                    })
                    .collect();
                Type::Concrete(TypeAnnotation::Object(resolved_fields))
            }
            TypeAnnotation::Generic { name, args } => {
                let resolved_args: Vec<_> = args
                    .iter()
                    .map(|a| self.resolve_type_annotation(a))
                    .collect();
                Type::Generic {
                    base: Box::new(Type::Concrete(TypeAnnotation::Reference(name.clone()))),
                    args: resolved_args,
                }
            }
            TypeAnnotation::Function { params, returns } => {
                let param_types: Vec<_> = params
                    .iter()
                    .map(|p| self.resolve_type_annotation(&p.type_annotation))
                    .collect();
                let return_type = self.resolve_type_annotation(returns);
                Type::Function {
                    params: param_types,
                    returns: Box::new(return_type),
                }
            }
            TypeAnnotation::Union(types) => {
                let resolved: Vec<TypeAnnotation> = types
                    .iter()
                    .filter_map(|t| {
                        self.resolve_type_annotation(t)
                            .to_annotation()
                    })
                    .collect();
                Type::Concrete(TypeAnnotation::Union(resolved))
            }
            TypeAnnotation::Intersection(types) => {
                let resolved: Vec<TypeAnnotation> = types
                    .iter()
                    .filter_map(|t| {
                        self.resolve_type_annotation(t)
                            .to_annotation()
                    })
                    .collect();
                Type::Concrete(TypeAnnotation::Intersection(resolved))
            }
            _ => Type::Concrete(ann.clone()),
        }
    }

    /// Register a trait definition in the type environment
    fn register_trait(&mut self, trait_def: &shape_ast::ast::TraitDef) -> TypeResult<()> {
        self.env.define_trait(trait_def);
        Ok(())
    }

    /// Register an impl block: validate against trait, register methods in MethodTable
    fn register_impl(&mut self, impl_block: &shape_ast::ast::ImplBlock) -> TypeResult<()> {
        let type_name = Self::type_name_str(&impl_block.target_type);
        let trait_name = Self::type_name_str(&impl_block.trait_name);

        self.validate_conversion_impl_shape(impl_block)?;

        let method_names: Vec<String> = impl_block.methods.iter().map(|m| m.name.clone()).collect();

        // Collect associated type bindings from the impl block
        let associated_types: std::collections::HashMap<String, TypeAnnotation> = impl_block
            .associated_type_bindings
            .iter()
            .map(|b| (b.name.clone(), b.concrete_type.clone()))
            .collect();

        // Validate impl methods against trait definition (arity check)
        if let Some(trait_def) = self.env.lookup_trait(&trait_name) {
            let trait_def = trait_def.clone();
            for member in &trait_def.members {
                let (trait_method_name, trait_arity) = match member {
                    TraitMember::Required(InterfaceMember::Method { name, params, .. }) => {
                        (name.as_str(), params.len())
                    }
                    TraitMember::Default(method_def) => {
                        (method_def.name.as_str(), method_def.params.len())
                    }
                    _ => continue,
                };

                // If the impl provides an override, check arity matches
                if let Some(impl_method) = impl_block
                    .methods
                    .iter()
                    .find(|m| m.name == trait_method_name)
                {
                    let impl_arity = impl_method.params.len();
                    if trait_arity != impl_arity {
                        return Err(TypeError::TraitImplArityMismatch {
                            trait_name: trait_name.clone(),
                            method_name: trait_method_name.to_string(),
                            expected: trait_arity,
                            got: impl_arity,
                        });
                    }
                }
            }
        }

        // Register the impl in the type registry (validates required methods +
        // associated types present, supertraits, and coherence)
        if let Err(msg) = self.env.register_trait_impl_with_assoc_types_named(
            &trait_name,
            &type_name,
            impl_block.impl_name.as_deref(),
            method_names,
            associated_types,
        ) {
            return Err(TypeError::TraitImplValidation(msg));
        }

        // Register each impl method in the method table under the target type
        let impl_method_names: Vec<String> =
            impl_block.methods.iter().map(|m| m.name.clone()).collect();

        for method in &impl_block.methods {
            let param_types: Vec<Type> = method
                .params
                .iter()
                .map(|p| {
                    if let Some(ann) = &p.type_annotation {
                        self.resolve_type_annotation(ann)
                    } else {
                        Type::Variable(TypeVar::fresh())
                    }
                })
                .collect();
            let return_type = method
                .return_type
                .as_ref()
                .map(|ann| self.resolve_type_annotation(ann))
                .unwrap_or_else(|| Type::Variable(TypeVar::fresh()));

            self.method_table.register_user_method(
                &type_name,
                &method.name,
                param_types,
                return_type,
            );
        }

        // Register default methods from the trait that the impl doesn't override
        if let Some(trait_def) = self.env.lookup_trait(&trait_name) {
            let trait_def = trait_def.clone();
            for member in &trait_def.members {
                if let TraitMember::Default(default_method) = member {
                    if !impl_method_names.contains(&default_method.name) {
                        let param_types: Vec<Type> = default_method
                            .params
                            .iter()
                            .map(|p| {
                                if let Some(ann) = &p.type_annotation {
                                    self.resolve_type_annotation(ann)
                                } else {
                                    Type::Variable(TypeVar::fresh())
                                }
                            })
                            .collect();
                        let return_type = default_method
                            .return_type
                            .as_ref()
                            .map(|ann| self.resolve_type_annotation(ann))
                            .unwrap_or_else(|| Type::Variable(TypeVar::fresh()));

                        self.method_table.register_user_method(
                            &type_name,
                            &default_method.name,
                            param_types,
                            return_type,
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Register extend block methods in the method table
    fn register_extend(&mut self, extend: &shape_ast::ast::ExtendStatement) -> TypeResult<()> {
        let type_name = Self::type_name_str(&extend.type_name);
        let targets = Self::extend_target_names(&type_name);

        for method in &extend.methods {
            let param_types: Vec<Type> = method
                .params
                .iter()
                .map(|p| {
                    if let Some(ann) = &p.type_annotation {
                        self.resolve_type_annotation(ann)
                    } else {
                        Type::Variable(TypeVar::fresh())
                    }
                })
                .collect();
            let return_type = method
                .return_type
                .as_ref()
                .map(|ann| self.resolve_type_annotation(ann))
                .unwrap_or_else(|| Type::Variable(TypeVar::fresh()));

            for target in &targets {
                self.method_table.register_user_method(
                    target,
                    &method.name,
                    param_types.clone(),
                    return_type.clone(),
                );
            }
        }

        Ok(())
    }

    /// Extract the simple type name string from a TypeName
    fn type_name_str(tn: &TypeName) -> String {
        match tn {
            TypeName::Simple(n) => n.clone(),
            TypeName::Generic { name, .. } => name.clone(),
        }
    }

    fn canonical_conversion_name_for_impl(name: &str) -> String {
        BuiltinTypes::canonical_script_alias(name)
            .map(ToString::to_string)
            .unwrap_or_else(|| name.to_string())
    }

    fn conversion_name_from_annotation_for_impl(annotation: &TypeAnnotation) -> Option<String> {
        match annotation {
            TypeAnnotation::Basic(name)
            | TypeAnnotation::Reference(name)
            | TypeAnnotation::Generic { name, .. } => {
                Some(Self::canonical_conversion_name_for_impl(name))
            }
            _ => None,
        }
    }

    fn validate_conversion_impl_shape(
        &self,
        impl_block: &shape_ast::ast::ImplBlock,
    ) -> TypeResult<()> {
        let trait_target = match &impl_block.trait_name {
            TypeName::Generic { name, type_args } if name == "TryInto" || name == "Into" => {
                if type_args.len() != 1 {
                    return Err(TypeError::TraitImplValidation(format!(
                        "{} impl must declare exactly one target: `impl {}<Target> for Source as target`",
                        name, name
                    )));
                }
                let target = Self::conversion_name_from_annotation_for_impl(&type_args[0])
                    .ok_or_else(|| {
                        TypeError::TraitImplValidation(format!(
                            "{} target must be a concrete named type",
                            name
                        ))
                    })?;
                Some((name.as_str(), target))
            }
            TypeName::Simple(name) if name == "TryInto" || name == "Into" => {
                return Err(TypeError::TraitImplValidation(format!(
                    "{} impl must use generic target form: `impl {}<Target> for Source as target`",
                    name, name
                )));
            }
            _ => None,
        };

        if let Some((trait_name, target)) = trait_target {
            let selector = impl_block.impl_name.as_deref().ok_or_else(|| {
                TypeError::TraitImplValidation(format!(
                    "{} impl must declare named selector with `as target`",
                    trait_name
                ))
            })?;
            let selector = Self::canonical_conversion_name_for_impl(selector);
            if selector != target {
                return Err(TypeError::TraitImplValidation(format!(
                    "{} target `{}` must match impl selector `{}`",
                    trait_name, target, selector
                )));
            }
        }

        Ok(())
    }

    fn extend_target_names(type_name: &str) -> Vec<String> {
        if BuiltinTypes::is_number_type_name(type_name) {
            // Numeric extensions should apply to both literal ints and widened numbers.
            return vec!["number".to_string(), "int".to_string()];
        }
        if BuiltinTypes::is_integer_type_name(type_name) {
            return vec!["int".to_string()];
        }
        if BuiltinTypes::is_string_type_name(type_name) {
            return vec!["string".to_string()];
        }
        if BuiltinTypes::is_bool_type_name(type_name) {
            return vec!["bool".to_string()];
        }
        vec![type_name.to_string()]
    }

    /// Build a TypeScheme for a function, including trait bounds from type params
    /// and where clause predicates
    fn make_function_scheme(&self, func: &FunctionDef, func_type: Type) -> TypeScheme {
        if let Some(type_params) = &func.type_params {
            let quantified: Vec<_> = type_params
                .iter()
                .map(|tp| TypeVar::new(tp.name.clone()))
                .collect();

            let mut bounds = std::collections::HashMap::new();
            let mut defaults = std::collections::HashMap::new();

            // Collect inline bounds from type params: <T: Comparable>
            for tp in type_params {
                if !tp.trait_bounds.is_empty() {
                    bounds.insert(tp.name.clone(), tp.trait_bounds.clone());
                }
                if let Some(default_ann) = &tp.default_type {
                    defaults.insert(tp.name.clone(), self.resolve_type_annotation(default_ann));
                }
            }

            // Merge where clause predicates: where T: Display + Serializable
            if let Some(where_preds) = &func.where_clause {
                for pred in where_preds {
                    bounds
                        .entry(pred.type_name.clone())
                        .or_insert_with(Vec::new)
                        .extend(pred.bounds.clone());
                }
            }

            if bounds.is_empty() && defaults.is_empty() {
                TypeScheme::poly(quantified, func_type)
            } else {
                TypeScheme::poly_bounded_with_defaults(quantified, func_type, bounds, defaults)
            }
        } else {
            self.env.generalize(&func_type)
        }
    }

    /// Infer type of variable declaration
    pub(crate) fn infer_variable_decl(&mut self, decl: &VariableDecl) -> TypeResult<Type> {
        let inferred_init_type = if let Some(init_expr) = &decl.value {
            Some(self.infer_expr(init_expr)?)
        } else {
            None
        };

        let declared_type = if let Some(ann) = &decl.type_annotation {
            self.resolve_type_annotation(ann)
        } else if let Some(inferred) = inferred_init_type.clone() {
            // When no annotation is provided, keep the inferred initializer type
            // so subsequent expressions can immediately use structural info.
            inferred
        } else {
            Type::Variable(TypeVar::fresh())
        };

        if let Some(inferred_type) = inferred_init_type {
            // Only add a constraint when an explicit annotation exists.
            // For unannotated declarations we already use the inferred type directly.
            if decl.type_annotation.is_some() {
                self.constraints
                    .push((declared_type.clone(), inferred_type));
            }
        }

        // For const, the type must be fully known
        if decl.kind == VarKind::Const && matches!(declared_type, Type::Variable(_)) {
            if let Some(name) = decl.pattern.as_identifier() {
                return Err(TypeError::ConstWithoutType(name.to_string()));
            } else {
                return Err(TypeError::ConstWithoutType("(destructured)".to_string()));
            }
        }

        if let Some(name) = decl.pattern.as_identifier() {
            self.env
                .define(name, TypeScheme::mono(declared_type.clone()));
        } else {
            self.bind_decl_pattern(&decl.pattern, declared_type.clone());
        }

        Ok(declared_type)
    }

    fn bind_decl_pattern(&mut self, pattern: &DestructurePattern, fallback_type: Type) {
        match pattern {
            DestructurePattern::Identifier(name, _) => {
                self.env.define(name, TypeScheme::mono(fallback_type));
            }
            DestructurePattern::Decomposition(bindings) => {
                for binding in bindings {
                    let binding_type = self.resolve_type_annotation(&binding.type_annotation);
                    self.env
                        .define(&binding.name, TypeScheme::mono(binding_type));
                }
            }
            DestructurePattern::Array(patterns) => {
                for pattern in patterns {
                    self.bind_decl_pattern(pattern, Type::Variable(TypeVar::fresh()));
                }
            }
            DestructurePattern::Object(fields) => {
                for field in fields {
                    self.bind_decl_pattern(&field.pattern, Type::Variable(TypeVar::fresh()));
                }
            }
            DestructurePattern::Rest(pattern) => {
                self.bind_decl_pattern(
                    pattern,
                    BuiltinTypes::array(Type::Variable(TypeVar::fresh())),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_system::inference::TypeInferenceEngine;

    #[test]
    fn test_trait_registration_during_inference() {
        use shape_ast::parser::parse_program;

        // Trait members use interface syntax: name(params): ReturnType
        let code = r#"
            trait Displayable {
                format(value: string): string
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Trait definition should type-check: {:?}",
            result.err()
        );

        // Trait should be registered in the environment
        let trait_def = engine.env.lookup_trait("Displayable");
        assert!(
            trait_def.is_some(),
            "Displayable trait should be registered"
        );
        assert_eq!(trait_def.unwrap().members.len(), 1);
    }

    #[test]
    fn test_impl_registers_methods_in_method_table() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Filterable {
                apply(pred: number): number
            }

            impl Filterable for Table {
                method apply(pred: number) {
                    return pred
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Impl block should type-check: {:?}",
            result.err()
        );

        // Method should be registered in the method table
        let table_type = Type::Concrete(TypeAnnotation::Reference("Table".to_string()));
        let sig = engine.method_table.lookup(&table_type, "apply");
        assert!(
            sig.is_some(),
            "apply method should be in method table for Table"
        );
    }

    #[test]
    fn test_into_impl_requires_generic_target_form() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Into<Target> {
                into(): Target
            }

            impl Into for string as int {
                method into() {
                    return 0
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Non-generic Into impl should produce validation error"
        );
    }

    #[test]
    fn test_into_impl_selector_must_match_target() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Into<Target> {
                into(): Target
            }

            impl Into<int> for string as number {
                method into() {
                    return 0
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Mismatched Into selector should produce validation error"
        );
    }

    #[test]
    fn test_impl_missing_required_method_errors() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Queryable {
                filter(pred: number): number;
                execute(): number
            }

            impl Queryable for Table {
                method filter(pred: number) {
                    return pred
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Missing required method should produce error"
        );

        let err = result.unwrap_err();
        match err {
            TypeError::TraitImplValidation(msg) => {
                assert!(
                    msg.contains("missing required method 'execute'"),
                    "Error should mention missing method: {}",
                    msg
                );
            }
            other => panic!("Expected TraitImplValidation, got: {:?}", other),
        }
    }

    #[test]
    fn test_impl_wrong_arity_errors() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Computable {
                compute(a: number, b: number): number
            }

            impl Computable for Calculator {
                method compute(a: number) {
                    return a
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(result.is_err(), "Wrong arity impl should produce error");

        let err = result.unwrap_err();
        match err {
            TypeError::TraitImplArityMismatch {
                trait_name,
                method_name,
                expected,
                got,
            } => {
                assert_eq!(trait_name, "Computable");
                assert_eq!(method_name, "compute");
                assert_eq!(expected, 2);
                assert_eq!(got, 1);
            }
            other => panic!("Expected TraitImplArityMismatch, got: {:?}", other),
        }
    }

    #[test]
    fn test_extend_registers_methods_in_method_table() {
        use shape_ast::parser::parse_program;

        let code = r#"
            extend Table<Row> {
                method smooth(window: number) {
                    return window
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Extend block should type-check: {:?}",
            result.err()
        );

        // Method should be registered
        let table_type = Type::Concrete(TypeAnnotation::Reference("Table".to_string()));
        assert!(
            engine.method_table.lookup(&table_type, "smooth").is_some(),
            "smooth method should be in method table for Table"
        );
    }

    #[test]
    fn test_extend_number_applies_to_int_receiver() {
        use shape_ast::parser::parse_program;

        let code = r#"
            extend Number {
                method double() {
                    return this * 2
                }
            }

            let x = 5.double()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);

        assert!(
            result.is_ok(),
            "Number extension should apply to int receivers: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_hasmethod_enforcement_known_method_passes() {
        use shape_ast::parser::parse_program;

        // Call a method that exists on the builtin type "string"
        let code = r#"
            let s: string = "hello"
            let n = s.len()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Calling existing method should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_hasmethod_enforcement_unknown_method_errors() {
        use shape_ast::parser::parse_program;

        // Call a method that does NOT exist on "string"
        let code = r#"
            let s: string = "hello"
            let x = s.nonExistentMethod()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Calling non-existent method on known type should produce error"
        );

        let err = result.unwrap_err();
        match err {
            TypeError::MethodNotFound {
                type_name,
                method_name,
            } => {
                assert_eq!(type_name, "string");
                assert_eq!(method_name, "nonExistentMethod");
            }
            other => panic!("Expected MethodNotFound, got: {:?}", other),
        }
    }

    #[test]
    fn test_impl_method_callable_after_registration() {
        use shape_ast::parser::parse_program;

        // Define a trait, implement it, then verify the method is callable on Person
        let code = r#"
            trait Greetable {
                greet(name: string): string
            }

            impl Greetable for Person {
                method greet(name: string) {
                    return name
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Impl block should type-check: {:?}",
            result.err()
        );

        // Verify the method is registered and callable on Person
        let person_type = Type::Concrete(TypeAnnotation::Reference("Person".to_string()));
        let sig = engine.method_table.lookup(&person_type, "greet");
        assert!(
            sig.is_some(),
            "greet method should be in method table for Person after impl registration"
        );
    }

    // ===== Sprint 2: ImplementsTrait Constraint + Parser Bounds =====

    #[test]
    fn test_parse_trait_bound_single() {
        use shape_ast::parser::parse_program;

        let code = r#"
            function identity<T: Comparable>(x: T) -> T {
                return x
            }
        "#;

        let program = parse_program(code).expect("Failed to parse trait bound syntax");
        if let shape_ast::ast::Item::Function(func, _) = &program.items[0] {
            let tp = &func.type_params.as_ref().unwrap()[0];
            assert_eq!(tp.name, "T");
            assert_eq!(tp.trait_bounds, vec!["Comparable".to_string()]);
        } else {
            panic!("Expected function item");
        }
    }

    #[test]
    fn test_parse_trait_bound_multiple() {
        use shape_ast::parser::parse_program;

        let code = r#"
            function display<T: Comparable + Displayable>(x: T) -> string {
                return "ok"
            }
        "#;

        let program = parse_program(code).expect("Failed to parse multiple trait bounds");
        if let shape_ast::ast::Item::Function(func, _) = &program.items[0] {
            let tp = &func.type_params.as_ref().unwrap()[0];
            assert_eq!(tp.name, "T");
            assert_eq!(
                tp.trait_bounds,
                vec!["Comparable".to_string(), "Displayable".to_string()]
            );
        } else {
            panic!("Expected function item");
        }
    }

    #[test]
    fn test_trait_bound_satisfied_passes() {
        use shape_ast::parser::parse_program;

        // Define a trait, implement it for number, then call a bounded function with number
        let code = r#"
            trait Comparable {
                compare(other: number): number
            }

            impl Comparable for number {
                method compare(other: number) {
                    return other
                }
            }

            function sort<T: Comparable>(x: T) -> T {
                return x
            }

            let result = sort(42)
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Calling bounded function with type that implements trait should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_trait_bound_violated_errors() {
        use shape_ast::parser::parse_program;

        // Define a trait but DON'T implement it for string, then call bounded function with string
        let code = r#"
            trait Sortable {
                rank(): number
            }

            function sort<T: Sortable>(x: T) -> T {
                return x
            }

            let result = sort("hello")
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Calling bounded function with type that does NOT implement trait should error"
        );

        let err = result.unwrap_err();
        match err {
            TypeError::TraitBoundViolation {
                type_name,
                trait_name,
            } => {
                assert_eq!(trait_name, "Sortable");
                assert_eq!(type_name, "string");
            }
            other => panic!("Expected TraitBoundViolation, got: {:?}", other),
        }
    }

    #[test]
    fn test_trait_bound_multiple_bounds_both_satisfied() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Comparable {
                compare(other: number): number
            }

            trait Displayable {
                display(): string
            }

            impl Comparable for number {
                method compare(other: number) {
                    return other
                }
            }

            impl Displayable for number {
                method display() {
                    return "num"
                }
            }

            function show_sorted<T: Comparable + Displayable>(x: T) -> T {
                return x
            }

            let result = show_sorted(42)
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Both trait bounds satisfied should pass: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_trait_bound_method_call_inside_generic_function() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Displayable {
                display(): string
            }

            type User { name: string }

            impl Displayable for User {
                method display() { "user:" + self.name }
            }

            fn render<T: Displayable>(value: T) -> string {
                value.display()
            }

            let out = render(User { name: "Ada" })
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Trait-bound method dispatch inside generic function should type-check: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_trait_bound_multiple_bounds_one_missing() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Comparable {
                compare(other: number): number
            }

            trait Displayable {
                display(): string
            }

            impl Comparable for number {
                method compare(other: number) {
                    return other
                }
            }

            function show_sorted<T: Comparable + Displayable>(x: T) -> T {
                return x
            }

            let result = show_sorted(42)
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Missing one of multiple trait bounds should error"
        );

        let err = result.unwrap_err();
        match err {
            TypeError::TraitBoundViolation { trait_name, .. } => {
                assert_eq!(trait_name, "Displayable");
            }
            other => panic!("Expected TraitBoundViolation, got: {:?}", other),
        }
    }

    // ===== Sprint 4: Default Methods + Display Trait =====

    #[test]
    fn test_parse_trait_with_default_method() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Displayable {
                format(): string;
                method describe() -> string {
                    return "object"
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse trait with default method");
        if let shape_ast::ast::Item::Trait(def, _) = &program.items[0] {
            assert_eq!(def.name, "Displayable");
            assert_eq!(def.members.len(), 2);
            assert!(matches!(&def.members[0], TraitMember::Required(_)));
            assert!(matches!(&def.members[1], TraitMember::Default(_)));
            if let TraitMember::Default(method) = &def.members[1] {
                assert_eq!(method.name, "describe");
            }
        } else {
            panic!("Expected trait item");
        }
    }

    #[test]
    fn test_default_method_used_when_impl_omits() {
        use shape_ast::parser::parse_program;

        // Define trait with a default method, impl without overriding it
        let code = r#"
            trait Printable {
                format(): string;
                method describe() -> string {
                    return "default"
                }
            }

            impl Printable for Widget {
                method format() -> string {
                    return "widget"
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Impl with default methods should type-check: {:?}",
            result.err()
        );

        // The default method "describe" should be registered on Widget
        let widget_type = Type::Concrete(TypeAnnotation::Reference("Widget".to_string()));
        assert!(
            engine
                .method_table
                .lookup(&widget_type, "describe")
                .is_some(),
            "Default method 'describe' should be in method table for Widget"
        );
        // The explicit method should also be there
        assert!(
            engine.method_table.lookup(&widget_type, "format").is_some(),
            "Explicit method 'format' should be in method table for Widget"
        );
    }

    #[test]
    fn test_default_method_overridden_by_impl() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Printable {
                format(): string;
                method describe() -> string {
                    return "default"
                }
            }

            impl Printable for Button {
                method format() -> string {
                    return "button"
                }
                method describe() -> string {
                    return "a button"
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Impl overriding default methods should type-check: {:?}",
            result.err()
        );

        // Both methods should be registered
        let button_type = Type::Concrete(TypeAnnotation::Reference("Button".to_string()));
        assert!(
            engine.method_table.lookup(&button_type, "format").is_some(),
            "format should be in method table for Button"
        );
        assert!(
            engine
                .method_table
                .lookup(&button_type, "describe")
                .is_some(),
            "describe should be in method table for Button"
        );
    }

    #[test]
    fn test_impl_missing_required_but_has_default() {
        use shape_ast::parser::parse_program;

        // Missing the required "format" method should still error
        let code = r#"
            trait Printable {
                format(): string;
                method describe() -> string {
                    return "default"
                }
            }

            impl Printable for Label {
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Missing required method should still error even when defaults exist"
        );
    }

    #[test]
    fn test_trait_all_defaults_no_impl_methods_needed() {
        use shape_ast::parser::parse_program;

        // Trait with only default methods — empty impl body should work
        let code = r#"
            trait HasDefaults {
                method greet() -> string {
                    return "hello"
                }
                method goodbye() -> string {
                    return "bye"
                }
            }

            impl HasDefaults for MyType {
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Trait with all defaults should allow empty impl: {:?}",
            result.err()
        );

        // Default methods should be registered
        let my_type = Type::Concrete(TypeAnnotation::Reference("MyType".to_string()));
        assert!(
            engine.method_table.lookup(&my_type, "greet").is_some(),
            "Default greet should be in method table for MyType"
        );
        assert!(
            engine.method_table.lookup(&my_type, "goodbye").is_some(),
            "Default goodbye should be in method table for MyType"
        );
    }

    #[test]
    fn test_trait_bound_nonexistent_trait_errors() {
        use shape_ast::parser::parse_program;

        let code = r#"
            function check<T: NonExistentTrait>(x: T) -> T {
                return x
            }

            let result = check(42)
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Using a non-existent trait bound should produce an error"
        );

        let err = result.unwrap_err();
        match err {
            TypeError::TraitBoundViolation { trait_name, .. } => {
                assert_eq!(trait_name, "NonExistentTrait");
            }
            other => panic!("Expected TraitBoundViolation, got: {:?}", other),
        }
    }

    #[test]
    fn test_decomposition_let_binds_named_variables_for_inference() {
        use shape_ast::parser::parse_program;

        let code = r#"
            type TypeA { x: int, y: int }
            type TypeB { z: int }

            let c = { x: 1, y: 2, z: 3 }
            let (f: TypeA, g: TypeB) = c as (TypeA + TypeB)
            let fx = f.x
            let gz = g.z
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let (_types, errors) = engine.infer_program_best_effort(&program);

        for error in errors {
            if let TypeError::UndefinedVariable(name) = error
                && (name == "f" || name == "g")
            {
                panic!(
                    "decomposition bindings should be defined, got undefined '{}'",
                    name
                );
            }
        }
    }
}
