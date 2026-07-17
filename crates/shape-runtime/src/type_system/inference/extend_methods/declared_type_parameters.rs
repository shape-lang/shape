//! Declared generic-parameter provenance for extend methods.

use super::TypeInferenceEngine;
use crate::type_system::checking::method_table::TypeParamExpr;
use crate::type_system::*;
use shape_ast::ast::{FunctionDef, MethodDef, TypeAnnotation};

impl TypeInferenceEngine {
    pub(super) fn register_extend_declared_type_parameters(
        &mut self,
        targets: &[String],
        method: &MethodDef,
        receiver_type_params: &[String],
        method_type_params: &[String],
    ) -> TypeResult<()> {
        for target in targets {
            self.register_declared_method_type_parameters(
                &format!("{}.{}", target, method.name),
                method,
                receiver_type_params,
                method_type_params,
            )?;
        }
        Ok(())
    }

    pub(super) fn infer_extend_method_with_declared_parameters(
        &mut self,
        method: &MethodDef,
        type_name: &str,
        receiver_param_names: &[String],
        func: &FunctionDef,
    ) -> TypeResult<()> {
        let registered = if func.type_params.is_some() {
            Some(self.declared_type_parameters_for_method(method)?)
        } else {
            None
        };
        let func_type = match registered.as_ref() {
            Some((declared, _)) => {
                self.infer_function_with_declared_parameter_capability(func, declared)?
            }
            None => self.infer_function(func)?,
        };
        if method.return_type.is_none() && !receiver_param_names.is_empty() {
            let Some((declared, receiver_count)) = registered.as_ref() else {
                return Err(TypeError::ConstraintViolation(
                    "inferred generic extend method lost its declaration capability".to_string(),
                ));
            };
            if declared.len() < *receiver_count {
                return Err(TypeError::ConstraintViolation(
                    "inferred extend method lost receiver generic-parameter provenance".to_string(),
                ));
            }
            let (receiver_tokens, method_tokens) = declared.split_at(*receiver_count);
            self.refresh_extend_method_return_from_body(
                type_name,
                method,
                receiver_param_names,
                receiver_tokens,
                method_tokens,
                &func_type,
            );
        }

        Ok(())
    }

    fn refresh_extend_method_return_from_body(
        &mut self,
        type_name: &str,
        method: &MethodDef,
        receiver_type_params: &[String],
        receiver_parameter_tokens: &[TypeVar],
        method_parameter_tokens: &[TypeVar],
        func_type: &Type,
    ) {
        let Type::Function { returns, .. } = func_type else {
            return;
        };
        let method_type_params: Vec<String> = method
            .type_params
            .as_ref()
            .map(|tps| tps.iter().map(|tp| tp.name().to_string()).collect())
            .unwrap_or_default();
        let Some(return_expr) = Self::type_to_extend_type_param_expr(
            returns,
            receiver_parameter_tokens,
            method_parameter_tokens,
        ) else {
            return;
        };
        let param_exprs: Vec<TypeParamExpr> = method
            .params
            .iter()
            .map(|p| match &p.type_annotation {
                Some(ann) => Self::annotation_to_type_param_expr(
                    ann,
                    receiver_type_params,
                    &method_type_params,
                ),
                None => TypeParamExpr::Concrete(self.fresh_type_var()),
            })
            .collect();

        for target in Self::extend_target_names(type_name) {
            self.method_table.register_user_generic_method(
                &target,
                &method.name,
                method_type_params.len(),
                param_exprs.clone(),
                return_expr.clone(),
                vec![],
            );
        }
    }

    fn type_to_extend_type_param_expr(
        ty: &Type,
        receiver_params: &[TypeVar],
        method_params: &[TypeVar],
    ) -> Option<TypeParamExpr> {
        match ty {
            Type::Variable(var) | Type::Constrained { var, .. } => {
                if let Some(idx) = receiver_params.iter().position(|declared| declared == var) {
                    Some(TypeParamExpr::ReceiverParam(idx))
                } else {
                    method_params
                        .iter()
                        .position(|declared| declared == var)
                        .map(TypeParamExpr::MethodParam)
                }
            }
            Type::Concrete(ann) => {
                if let Some(variable) = annotation_as_tyvar(ann) {
                    return Self::type_to_extend_type_param_expr(
                        &Type::Variable(variable),
                        receiver_params,
                        method_params,
                    );
                }
                Some(TypeParamExpr::Concrete(Type::Concrete(ann.clone())))
            }
            Type::Generic { base, args } => {
                let name = match base.as_ref() {
                    Type::Concrete(TypeAnnotation::Reference(path)) => path.to_string(),
                    Type::Concrete(TypeAnnotation::Basic(name)) => name.clone(),
                    _ => return Some(TypeParamExpr::Concrete(ty.clone())),
                };
                Some(TypeParamExpr::GenericContainer {
                    name,
                    args: args
                        .iter()
                        .map(|arg| {
                            Self::type_to_extend_type_param_expr(
                                arg,
                                receiver_params,
                                method_params,
                            )
                        })
                        .collect::<Option<Vec<_>>>()?,
                })
            }
            Type::Function { params, returns } => Some(TypeParamExpr::Function {
                params: params
                    .iter()
                    .map(|param| {
                        Self::type_to_extend_type_param_expr(param, receiver_params, method_params)
                    })
                    .collect::<Option<Vec<_>>>()?,
                returns: Box::new(Self::type_to_extend_type_param_expr(
                    returns,
                    receiver_params,
                    method_params,
                )?),
            }),
        }
    }
}
