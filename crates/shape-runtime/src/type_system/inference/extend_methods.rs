//! Extend-block method registration and body inference.

use super::TypeInferenceEngine;
use crate::type_system::checking::method_table::TypeParamExpr;
use crate::type_system::*;
use shape_ast::ast::{
    DestructurePattern, ExtendStatement, FunctionDef, FunctionParameter, MethodDef, Span,
    TypeAnnotation, TypeName, TypeParam,
};

mod declared_type_parameters;

impl TypeInferenceEngine {
    /// Register extend block methods in the method table.
    ///
    /// For generic extend blocks (e.g., `extend Vec<T>`), methods that reference
    /// type parameters are registered as `GenericMethodSignature` entries with
    /// `TypeParamExpr` trees, enabling proper generic method resolution.
    pub(super) fn register_extend(&mut self, extend: &ExtendStatement) -> TypeResult<()> {
        let type_name = Self::type_name_str(&extend.type_name);
        let targets = Self::extend_target_names(&type_name);

        // Extract receiver type param names from generic extend blocks.
        // e.g., `extend Vec<T>` -> receiver_type_params = ["T"]
        // e.g., `extend HashMap<K, V>` -> receiver_type_params = ["K", "V"]
        let implicit_receiver_type_params =
            Self::implicit_extend_receiver_type_params(&extend.type_name);
        let mut receiver_type_params = implicit_receiver_type_params.clone();
        let explicit_receiver_type_params: Vec<String> = match &extend.type_name {
            TypeName::Generic { type_args, .. } => type_args
                .iter()
                .filter_map(|arg| {
                    let name_str = match arg {
                        TypeAnnotation::Basic(name) => name.as_str(),
                        TypeAnnotation::Reference(path) => path.as_str(),
                        _ => return None,
                    };
                    // Single uppercase letter or two-char uppercase = type param
                    let first = name_str.chars().next().unwrap_or('a');
                    if first.is_uppercase() && name_str.len() <= 2 {
                        Some(name_str.to_string())
                    } else {
                        None
                    }
                })
                .collect(),
            _ => vec![],
        };
        for param in explicit_receiver_type_params {
            if !receiver_type_params.contains(&param) {
                receiver_type_params.push(param);
            }
        }

        let has_receiver_params = !receiver_type_params.is_empty();

        for method in &extend.methods {
            // Extract method-level type params (e.g., `method map<U>(...)`)
            let method_type_params: Vec<String> = method
                .type_params
                .as_ref()
                .map(|tps| tps.iter().map(|tp| tp.name().to_string()).collect())
                .unwrap_or_default();

            // A scalar/collection extend synthesizes an implicit receiver type
            // param (Number's `N`, bare Array/Vec's `T`). That name is a
            // compiler artifact, not a user binder: when a method declares its
            // own type param of the same name, the explicit method binder
            // shadows the synthetic receiver, so drop the redeclared implicit
            // name before provenance registration counts it a second time.
            // Explicit user-written receiver params (e.g. `extend Vec<T>`) stay
            // intact and still collide as genuine shadows.
            let mut receiver_type_params = receiver_type_params.clone();
            receiver_type_params.retain(|name| {
                !(method_type_params.contains(name) && implicit_receiver_type_params.contains(name))
            });

            let is_generic = has_receiver_params || !method_type_params.is_empty();

            if is_generic {
                self.register_extend_declared_type_parameters(
                    &targets,
                    method,
                    &receiver_type_params,
                    &method_type_params,
                )?;
                // Build TypeParamExpr-based signature for generic method resolution.
                let param_exprs: Vec<TypeParamExpr> = method
                    .params
                    .iter()
                    .map(|p| match &p.type_annotation {
                        Some(ann) => Self::annotation_to_type_param_expr(
                            ann,
                            &receiver_type_params,
                            &method_type_params,
                        ),
                        None => TypeParamExpr::Concrete(self.fresh_type_var()),
                    })
                    .collect();
                let return_expr = match method.return_type.as_ref() {
                    Some(ann) => Self::annotation_to_type_param_expr(
                        ann,
                        &receiver_type_params,
                        &method_type_params,
                    ),
                    None => TypeParamExpr::Concrete(self.fresh_type_var()),
                };

                // Extract receiver param bounds from method-level type params
                // that reference receiver type params with trait bounds.
                // For now, bounds come from the extend block's type args if
                // they have trait bounds (via where clauses on the extend).
                let receiver_param_bounds: Vec<(usize, Vec<String>)> = vec![];

                for target in &targets {
                    self.method_table.register_user_generic_method(
                        target,
                        &method.name,
                        method_type_params.len(),
                        param_exprs.clone(),
                        return_expr.clone(),
                        receiver_param_bounds.clone(),
                    );
                }
            } else {
                // Non-generic: use the existing monomorphic registration.
                let param_types: Vec<Type> = method
                    .params
                    .iter()
                    .map(|p| match &p.type_annotation {
                        Some(ann) => self.resolve_type_annotation(ann),
                        None => self.fresh_type_var(),
                    })
                    .collect();
                let return_type = match method.return_type.as_ref() {
                    Some(ann) => self.resolve_type_annotation(ann),
                    None => self.fresh_type_var(),
                };

                for target in &targets {
                    self.method_table.register_user_method(
                        target,
                        &method.name,
                        param_types.clone(),
                        return_type.clone(),
                    );
                }
            }
        }

        Ok(())
    }

    /// Type-check extend method bodies with `self` in scope where the receiver
    /// type is statically knowable before a runtime value exists.
    ///
    /// `register_extend` publishes the callable surface, but strict expression
    /// proof also needs the body walk so field reads like `self.name` resolve
    /// from the target struct schema before overloaded `+` is checked.
    ///
    /// Bare `extend Vec` / `extend Array` methods are also checked as
    /// receiver-parametric `self: Vec<T>` / `Array<T>` templates. That lets
    /// `self[index]` carry the receiver element type through the existing
    /// index-access rules instead of falling to a disconnected fresh variable.
    /// The actual `T` remains a compile-time call-site proof supplied by the
    /// generic method resolver/monomorphizer; no runtime probing is involved.
    pub(super) fn infer_extend_method_bodies(
        &mut self,
        extend: &ExtendStatement,
    ) -> TypeResult<()> {
        let type_name = Self::type_name_str(&extend.type_name);
        let should_infer_body = self.struct_type_defs.contains_key(type_name.as_str())
            || Self::bare_single_param_collection_extend(&extend.type_name).is_some()
            || Self::scalar_extend_receiver_annotation(&extend.type_name).is_some();
        if !should_infer_body {
            return Ok(());
        }

        let self_ann = Self::type_name_to_annotation_for_extend_body(&extend.type_name);
        let receiver_type_params = Self::implicit_extend_type_params(&extend.type_name);
        let receiver_param_names = Self::implicit_extend_receiver_type_params(&extend.type_name);
        for method in &extend.methods {
            let func = Self::inference_function_for_extend_method(
                method,
                &type_name,
                &self_ann,
                &receiver_type_params,
            );
            self.infer_extend_method_with_declared_parameters(
                method,
                &type_name,
                &receiver_param_names,
                &func,
            )?;
        }

        Ok(())
    }

    /// Convert a type annotation to a TypeParamExpr, mapping type parameter names
    /// to ReceiverParam/MethodParam indices.
    ///
    /// For `extend Vec<T> { method map<U>(f: (T) => U): Vec<U> { ... } }`:
    /// - `T` -> `ReceiverParam(0)`
    /// - `U` -> `MethodParam(0)`
    /// - `(T) => U` -> `Function { params: [ReceiverParam(0)], returns: MethodParam(0) }`
    /// - `Vec<U>` -> `GenericContainer { name: "Vec", args: [MethodParam(0)] }`
    pub(super) fn annotation_to_type_param_expr(
        ann: &TypeAnnotation,
        receiver_params: &[String],
        method_params: &[String],
    ) -> TypeParamExpr {
        let check_param = |name_str: &str| -> Option<TypeParamExpr> {
            if let Some(idx) = receiver_params.iter().position(|p| p == name_str) {
                return Some(TypeParamExpr::ReceiverParam(idx));
            }
            if let Some(idx) = method_params.iter().position(|p| p == name_str) {
                return Some(TypeParamExpr::MethodParam(idx));
            }
            None
        };

        match ann {
            TypeAnnotation::Basic(name) => {
                if let Some(expr) = check_param(name.as_str()) {
                    return expr;
                }
                TypeParamExpr::Concrete(Type::Concrete(ann.clone()))
            }
            TypeAnnotation::Reference(path) => {
                if let Some(expr) = check_param(path.as_str()) {
                    return expr;
                }
                TypeParamExpr::Concrete(Type::Concrete(ann.clone()))
            }
            TypeAnnotation::Function { params, returns } => {
                let param_exprs: Vec<TypeParamExpr> = params
                    .iter()
                    .map(|p| {
                        Self::annotation_to_type_param_expr(
                            &p.type_annotation,
                            receiver_params,
                            method_params,
                        )
                    })
                    .collect();
                let return_expr = Box::new(Self::annotation_to_type_param_expr(
                    returns,
                    receiver_params,
                    method_params,
                ));
                TypeParamExpr::Function {
                    params: param_exprs,
                    returns: return_expr,
                }
            }
            TypeAnnotation::Generic { name, args } => {
                let name_str = name.as_str();
                if args.is_empty() {
                    if let Some(expr) = check_param(name_str) {
                        return expr;
                    }
                }
                let arg_exprs: Vec<TypeParamExpr> = args
                    .iter()
                    .map(|a| Self::annotation_to_type_param_expr(a, receiver_params, method_params))
                    .collect();
                TypeParamExpr::GenericContainer {
                    name: name_str.to_string(),
                    args: arg_exprs,
                }
            }
            TypeAnnotation::Array(elem) => {
                let elem_expr =
                    Self::annotation_to_type_param_expr(elem, receiver_params, method_params);
                TypeParamExpr::GenericContainer {
                    name: "Vec".to_string(),
                    args: vec![elem_expr],
                }
            }
            TypeAnnotation::Void => TypeParamExpr::Concrete(Type::Concrete(TypeAnnotation::Void)),
            _ => TypeParamExpr::Concrete(Type::Concrete(ann.clone())),
        }
    }

    fn inference_function_for_extend_method(
        method: &MethodDef,
        type_name: &str,
        self_ann: &TypeAnnotation,
        receiver_type_params: &[TypeParam],
    ) -> FunctionDef {
        let mut params = Vec::with_capacity(method.params.len() + 2);
        for receiver_name in ["self", "this"] {
            params.push(FunctionParameter {
                pattern: DestructurePattern::Identifier(receiver_name.to_string(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: Some(self_ann.clone()),
                default_value: None,
            });
        }
        params.extend(method.params.clone());

        let mut type_params = receiver_type_params.to_vec();
        if let Some(method_tps) = method.type_params.as_ref() {
            for tp in method_tps {
                let name = tp.name();
                if !type_params.iter().any(|existing| existing.name() == name) {
                    type_params.push(tp.clone());
                }
            }
        }

        FunctionDef {
            name: format!("{}.{}", type_name, method.name),
            name_span: method.span,
            declaring_module_path: method.declaring_module_path.clone(),
            doc_comment: None,
            params,
            return_type: method.return_type.clone(),
            body: method.body.clone(),
            type_params: if type_params.is_empty() {
                None
            } else {
                Some(type_params)
            },
            annotations: method.annotations.clone(),
            is_async: method.is_async,
            is_comptime: false,
            where_clause: None,
        }
    }

    fn type_name_to_annotation_for_extend_body(type_name: &TypeName) -> TypeAnnotation {
        if let Some(base) = Self::bare_single_param_collection_extend(type_name) {
            return TypeAnnotation::Generic {
                name: shape_ast::ast::type_path::TypePath::simple(base),
                args: vec![TypeAnnotation::Basic("T".to_string())],
            };
        }
        if let Some(ann) = Self::scalar_extend_receiver_annotation(type_name) {
            return ann;
        }
        Self::type_name_to_annotation_for_impl(type_name)
    }

    fn bare_single_param_collection_extend(type_name: &TypeName) -> Option<&str> {
        match type_name {
            TypeName::Simple(name) if matches!(name.as_str(), "Array" | "Vec") => {
                Some(name.as_str())
            }
            _ => None,
        }
    }

    fn implicit_extend_receiver_type_params(type_name: &TypeName) -> Vec<String> {
        if Self::bare_single_param_collection_extend(type_name).is_some() {
            vec!["T".to_string()]
        } else if Self::number_family_extend(type_name) {
            vec!["N".to_string()]
        } else {
            vec![]
        }
    }

    fn implicit_extend_type_params(type_name: &TypeName) -> Vec<TypeParam> {
        Self::implicit_extend_receiver_type_params(type_name)
            .into_iter()
            .map(|name| TypeParam::Type {
                name,
                span: Span::DUMMY,
                doc_comment: None,
                default_type: None,
                trait_bounds: Vec::new(),
            })
            .collect()
    }

    fn scalar_extend_receiver_annotation(type_name: &TypeName) -> Option<TypeAnnotation> {
        let TypeName::Simple(name) = type_name else {
            return None;
        };
        if Self::number_family_extend(type_name) {
            return Some(TypeAnnotation::Basic("N".to_string()));
        }
        BuiltinTypes::canonical_script_alias(name.as_str())
            .map(|alias| TypeAnnotation::Basic(alias.to_string()))
    }

    fn number_family_extend(type_name: &TypeName) -> bool {
        matches!(type_name, TypeName::Simple(name) if matches!(name.as_str(), "Number" | "number"))
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
}
