//! Compiler-issued declared-parameter capabilities and exact schemes.

use super::super::TypeInferenceEngine;
use crate::type_system::{Type, TypeError, TypeResult, TypeScheme, TypeVar};
use shape_ast::ast::{FunctionDef, TypeParam, WherePredicate};
use std::collections::{HashMap, HashSet};

impl TypeInferenceEngine {
    pub(super) fn mint_declared_type_params(&mut self, params: &[TypeParam]) -> Vec<TypeVar> {
        if params.is_empty() {
            return Vec::new();
        }
        let owner = self.type_var_gen.fresh_declared_owner();
        params
            .iter()
            .enumerate()
            .map(|(ordinal, param)| {
                let ordinal = u32::try_from(ordinal).expect("declared TypeVar ordinal overflow");
                TypeVar::declared(owner, ordinal, param.name())
            })
            .collect()
    }

    pub(super) fn validate_declared_type_param_vector(
        &self,
        callable_name: &str,
        params: &[TypeParam],
        vars: &[TypeVar],
    ) -> TypeResult<()> {
        if params.len() != vars.len() {
            return Err(TypeError::ConstraintViolation(format!(
                "internal inference error: `{callable_name}` declared parameter arity does not match its capability vector"
            )));
        }
        let owner = match vars.first() {
            Some(var) => Some(
                var.declared_provenance()
                    .ok_or_else(|| {
                        TypeError::ConstraintViolation(format!(
                            "internal inference error: `{callable_name}` has a non-declared generic parameter capability"
                        ))
                    })?
                    .owner(),
            ),
            None => None,
        };
        for (ordinal, (param, var)) in params.iter().zip(vars).enumerate() {
            let expected_ordinal = u32::try_from(ordinal).map_err(|_| {
                TypeError::ConstraintViolation(format!(
                    "internal inference error: `{callable_name}` declared parameter ordinal overflow"
                ))
            })?;
            let Some(provenance) = var.declared_provenance() else {
                return Err(TypeError::ConstraintViolation(format!(
                    "internal inference error: `{callable_name}` has a non-declared generic parameter capability"
                )));
            };
            if Some(provenance.owner()) != owner
                || provenance.ordinal() != expected_ordinal
                || provenance.source_name() != param.name()
            {
                return Err(TypeError::ConstraintViolation(format!(
                    "internal inference error: `{callable_name}` has a malformed declared parameter capability vector"
                )));
            }
        }
        Ok(())
    }

    pub(super) fn bind_validated_declared_type_params(
        &mut self,
        params: &[TypeParam],
        vars: &[TypeVar],
    ) {
        for (param, var) in params.iter().zip(vars) {
            self.env
                .define(param.name(), TypeScheme::mono(Type::Variable(var.clone())));
        }
    }

    pub(super) fn validate_declared_type_params_in_type(
        &self,
        func: &FunctionDef,
        ty: &Type,
        quantified: &[TypeVar],
    ) -> TypeResult<()> {
        let type_params = func.type_params.as_deref().unwrap_or_default();
        self.validate_declared_type_param_vector(&func.name, type_params, quantified)?;
        let mut embedded = HashSet::new();
        self.collect_type_vars(ty, &mut embedded);
        if embedded.iter().any(|var| {
            var.declared_provenance().is_some() && !quantified.iter().any(|known| known == var)
        }) {
            return Err(TypeError::ConstraintViolation(format!(
                "internal inference error: `{}` type contains a foreign declared parameter capability",
                func.name
            )));
        }
        Ok(())
    }

    pub(super) fn make_function_scheme(
        &mut self,
        func: &FunctionDef,
        func_type: Type,
    ) -> TypeResult<TypeScheme> {
        let Some(type_params) = func.type_params.as_deref() else {
            self.validate_declared_type_params_in_type(func, &func_type, &[])?;
            return Ok(self.env.generalize(&func_type));
        };
        let vars = self.declared_type_parameters_for_callable(func)?;
        self.make_function_scheme_with_params(func, func_type, &vars)
    }

    pub(super) fn make_function_scheme_with_params(
        &mut self,
        func: &FunctionDef,
        func_type: Type,
        quantified: &[TypeVar],
    ) -> TypeResult<TypeScheme> {
        self.validate_declared_type_params_in_type(func, &func_type, quantified)?;
        let Some(type_params) = func.type_params.as_deref() else {
            return Ok(self.env.generalize(&func_type));
        };
        self.make_declared_param_scheme(
            &func.name,
            type_params,
            func.where_clause.as_deref(),
            func_type,
            quantified,
        )
    }

    pub(super) fn make_declared_param_scheme(
        &mut self,
        callable_name: &str,
        type_params: &[TypeParam],
        where_clause: Option<&[WherePredicate]>,
        func_type: Type,
        quantified: &[TypeVar],
    ) -> TypeResult<TypeScheme> {
        self.validate_declared_type_param_vector(callable_name, type_params, quantified)?;
        self.env.push_scope();
        self.bind_validated_declared_type_params(type_params, quantified);

        let mut bounds: HashMap<TypeVar, Vec<String>> = HashMap::new();
        let mut defaults: HashMap<TypeVar, Type> = HashMap::new();
        for (tp, var) in type_params.iter().zip(quantified) {
            let trait_bounds = tp.trait_bounds();
            if !trait_bounds.is_empty() {
                let mut expanded: Vec<String> =
                    trait_bounds.iter().map(|t| t.to_string()).collect();
                for trait_name in trait_bounds {
                    let supers = self
                        .env
                        .get_transitive_supertrait_names(trait_name.as_str());
                    for st in supers {
                        if !expanded.contains(&st) {
                            expanded.push(st);
                        }
                    }
                }
                bounds.insert(var.clone(), expanded);
            }
            if let Some(default_ann) = tp.default_type() {
                defaults.insert(var.clone(), self.resolve_type_annotation(default_ann));
            }
        }

        if let Some(where_preds) = where_clause {
            for pred in where_preds {
                if let Some(var) = type_params
                    .iter()
                    .zip(quantified)
                    .find_map(|(param, var)| (param.name() == pred.type_name).then_some(var))
                {
                    let mut expanded: Vec<String> =
                        pred.bounds.iter().map(|t| t.to_string()).collect();
                    for trait_name in &pred.bounds {
                        let supers = self
                            .env
                            .get_transitive_supertrait_names(trait_name.as_str());
                        for st in supers {
                            if !expanded.contains(&st) {
                                expanded.push(st);
                            }
                        }
                    }
                    bounds
                        .entry(var.clone())
                        .or_insert_with(Vec::new)
                        .extend(expanded);
                }
            }
        }
        self.env.pop_scope();

        if bounds.is_empty() && defaults.is_empty() {
            Ok(TypeScheme::poly(quantified.to_vec(), func_type))
        } else {
            Ok(TypeScheme::poly_bounded_with_exact_defaults(
                quantified.to_vec(),
                func_type,
                bounds,
                defaults,
            ))
        }
    }
}
