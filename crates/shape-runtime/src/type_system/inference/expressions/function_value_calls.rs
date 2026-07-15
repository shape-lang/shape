//! Inference for statically known function-valued local bindings.

use super::TypeInferenceEngine;
use crate::type_system::*;
use shape_ast::ast::{Expr, TypeAnnotation};

impl TypeInferenceEngine {
    fn local_function_value_call_excluded(name: &str) -> bool {
        matches!(
            name,
            // Builtin/special forms keep their dedicated inference paths.
            "print"
                | "range"
                | "min"
                | "max"
                | "len"
                | "fold"
                | "HashMap"
                | "Set"
                | "Deque"
                | "PriorityQueue"
                | "Channel"
                | "Mutex"
                | "Atomic"
                | "Lazy"
                | "Some"
                | "Ok"
                | "Err"
        )
    }

    fn function_shape_for_value_call(func_type: Type) -> Option<(Vec<Type>, Type)> {
        match func_type {
            Type::Function { params, returns } => Some((params, returns.as_ref().clone())),
            Type::Concrete(TypeAnnotation::Function {
                params: concrete_params,
                returns: concrete_returns,
            }) => {
                let params = concrete_params
                    .iter()
                    .map(|param| match annotation_as_tyvar(&param.type_annotation) {
                        Some(var) => Type::Variable(var),
                        None => Type::Concrete(param.type_annotation.clone()),
                    })
                    .collect();
                let returns = match annotation_as_tyvar(&concrete_returns) {
                    Some(var) => Type::Variable(var),
                    None => Type::Concrete(*concrete_returns),
                };
                Some((params, returns))
            }
            _ => None,
        }
    }

    fn source_var_for_value_call_param(ty: &Type) -> Option<TypeVar> {
        match ty {
            Type::Variable(var) | Type::Constrained { var, .. } => Some(var.clone()),
            _ => None,
        }
    }

    pub(super) fn infer_function_value_binding_call(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Option<TypeResult<Type>> {
        if self.lookup_callable_origin_for_name(name).is_some()
            || Self::local_function_value_call_excluded(name)
            || crate::builtin_metadata::is_comptime_builtin_function(name)
        {
            return None;
        }

        let scheme = self.env.lookup(name).cloned()?;
        // Declared generics require the named-call path, which retains the
        // exact declared-token -> fresh-hole specialization evidence. This
        // compatibility path handles inference-hole function values only.
        if scheme
            .quantified
            .iter()
            .any(|variable| variable.declared_provenance().is_some())
        {
            return None;
        }

        let func_type = scheme.instantiate(&mut self.type_var_gen);
        let (params, returns) = Self::function_shape_for_value_call(func_type)?;
        if !params
            .iter()
            .any(|param| Self::source_var_for_value_call_param(param).is_some())
        {
            return None;
        }
        if params.len() != args.len() {
            return Some(Err(TypeError::ArityMismatch(params.len(), args.len())));
        }

        for (arg, param_ty) in args.iter().zip(params.iter()) {
            let arg_ty = match self.infer_expr(arg) {
                Ok(ty) => ty,
                Err(err) => return Some(Err(err)),
            };
            let expected = Self::adopt_int_literal_in_context(arg, param_ty)
                .or_else(|| {
                    Self::source_var_for_value_call_param(param_ty)
                        .and_then(|var| self.deferred_closure_numeric_param_body_hint.get(&var))
                        .and_then(|hint| Self::adopt_int_literal_in_context(arg, hint))
                })
                .unwrap_or_else(|| arg_ty.clone());
            self.constraints.push((expected.clone(), param_ty.clone()));

            if let Some(param_var) = Self::source_var_for_value_call_param(param_ty) {
                let resolved_expected = self.solver.unifier().apply_substitutions(&expected);
                if !self.type_contains_unresolved_vars(&resolved_expected)
                    && self.solver.unifier().lookup(&param_var).is_none()
                {
                    self.solver.unifier_mut().bind(param_var, resolved_expected);
                }
            }
        }
        Some(Ok(self.solver.unifier().apply_substitutions(&returns)))
    }
}
