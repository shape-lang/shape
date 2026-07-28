//! Exact declared-parameter evidence for qualified generic method calls.

use shape_ast::ast::{Expr, MethodDef, Span, TypeAnnotation};

use super::{SemanticCallSiteCandidate, SemanticCallSiteKey, SemanticDeclaredInstantiation};
use crate::type_system::{
    Type, TypeError, TypeInferenceEngine, TypeResult, TypeVar, tyvar_to_annotation,
};

/// Compiler-qualified method declaration capability retained across the
/// registration and expression-inference passes.
#[derive(Debug, Clone)]
pub(in crate::type_system::inference::generated_callable_facts) struct DeclaredMethodTypeParameters
{
    pub(super) source_names: Vec<String>,
    pub(super) receiver: Vec<TypeVar>,
    pub(super) method: Vec<TypeVar>,
}

/// Opaque identity of one immutable method declaration for one inference run.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::type_system::inference::generated_callable_facts) struct InferenceMethodDeclarationToken(
    usize,
);

impl InferenceMethodDeclarationToken {
    fn of(method: &MethodDef) -> Self {
        Self(std::ptr::from_ref(method) as usize)
    }
}

impl std::fmt::Debug for InferenceMethodDeclarationToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("InferenceMethodDeclarationToken(..)")
    }
}

impl TypeInferenceEngine {
    pub(in crate::type_system::inference) fn declared_type_parameters_for_method(
        &self,
        method: &MethodDef,
    ) -> TypeResult<(Vec<TypeVar>, usize)> {
        let declaration = InferenceMethodDeclarationToken::of(method);
        let parameters = self
            .generated_inference
            .method_declared_parameters
            .get(&declaration)
            .ok_or_else(|| {
                TypeError::ConstraintViolation(
                    "internal inference error: method declaration has no registered generic-token capability"
                        .to_string(),
                )
            })?;
        let mut declared = parameters.receiver.clone();
        declared.extend(parameters.method.iter().cloned());
        Ok((declared, parameters.receiver.len()))
    }

    pub(in crate::type_system::inference) fn register_declared_method_type_parameters(
        &mut self,
        qualified_name: &str,
        method: &MethodDef,
        receiver_names: &[String],
        method_names: &[String],
    ) -> TypeResult<()> {
        let source_names: Vec<_> = receiver_names.iter().chain(method_names).cloned().collect();
        if source_names.is_empty() {
            return Ok(());
        }
        let mut distinct = std::collections::HashSet::new();
        if let Some(duplicate) = source_names
            .iter()
            .find(|name| !distinct.insert((*name).clone()))
        {
            return Err(TypeError::ConstraintViolation(format!(
                "generic method '{qualified_name}' declares type parameter '{duplicate}' more than once"
            )));
        }
        let declaration = InferenceMethodDeclarationToken::of(method);
        if let Some(existing) = self
            .generated_inference
            .method_declared_parameters
            .get(&declaration)
        {
            if existing.source_names == source_names {
                self.generated_inference
                    .active_method_declarations
                    .insert(qualified_name.to_string(), declaration);
                return Ok(());
            }
            return Err(TypeError::ConstraintViolation(format!(
                "generic method '{qualified_name}' was registered with conflicting type parameter declarations"
            )));
        }

        let owner = self.type_var_gen.fresh_declared_owner();
        let declared: Vec<_> = source_names
            .iter()
            .enumerate()
            .map(|(ordinal, name)| {
                TypeVar::declared(
                    owner,
                    u32::try_from(ordinal).expect("method type parameter count exceeds u32"),
                    name,
                )
            })
            .collect();
        let (receiver, method) = declared.split_at(receiver_names.len());
        self.generated_inference.method_declared_parameters.insert(
            declaration,
            DeclaredMethodTypeParameters {
                source_names,
                receiver: receiver.to_vec(),
                method: method.to_vec(),
            },
        );
        self.generated_inference
            .active_method_declarations
            .insert(qualified_name.to_string(), declaration);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_semantic_method_call_arguments(
        &mut self,
        qualified_name: &str,
        call_span: Span,
        receiver: &Expr,
        receiver_type: &Type,
        receiver_parameters: &[Type],
        method_variables: &[Type],
        args: &[Expr],
        argument_types: &[Type],
        expected_argument_types: &[Type],
    ) {
        let Some(declaration) = self
            .generated_inference
            .active_method_declarations
            .get(qualified_name)
        else {
            return;
        };
        let Some(declared) = self
            .generated_inference
            .method_declared_parameters
            .get(declaration)
            .cloned()
        else {
            return;
        };
        if declared.receiver.len() != receiver_parameters.len()
            || declared.method.len() != method_variables.len()
            || args.len() != argument_types.len()
            || args.len() != expected_argument_types.len()
        {
            return;
        }

        let mut instantiations = Vec::with_capacity(
            declared
                .receiver
                .len()
                .saturating_add(declared.method.len()),
        );
        let mut receiver_variables = Vec::with_capacity(declared.receiver.len());
        for (parameter, actual) in declared.receiver.iter().zip(receiver_parameters) {
            let instantiated = self.type_var_gen.fresh_var();
            self.constraints
                .push((Type::Variable(instantiated.clone()), actual.clone()));
            receiver_variables.push(instantiated.clone());
            instantiations.push(SemanticDeclaredInstantiation::new(
                parameter.clone(),
                instantiated,
            ));
        }
        for (parameter, variable) in declared.method.iter().zip(method_variables) {
            let instantiated = match variable {
                Type::Variable(variable) | Type::Constrained { var: variable, .. } => {
                    variable.clone()
                }
                other => {
                    let instantiated = self.type_var_gen.fresh_var();
                    self.constraints
                        .push((Type::Variable(instantiated.clone()), other.clone()));
                    instantiated
                }
            };
            instantiations.push(SemanticDeclaredInstantiation::new(
                parameter.clone(),
                instantiated,
            ));
        }

        let Some(receiver_pattern) = receiver_parameter_pattern(receiver_type, &receiver_variables)
        else {
            return;
        };
        let mut parameter_types = Vec::with_capacity(1 + expected_argument_types.len());
        parameter_types.push(receiver_pattern);
        parameter_types.extend_from_slice(expected_argument_types);
        let mut arguments = Vec::with_capacity(1 + args.len());
        arguments.push(self.semantic_candidates_for_call_argument(receiver, receiver_type));
        arguments.extend(
            args.iter()
                .zip(argument_types)
                .map(|(argument, ty)| self.semantic_candidates_for_call_argument(argument, ty)),
        );

        let key = SemanticCallSiteKey::new(
            self.generated_inference.active_node_stack.last().cloned(),
            qualified_name,
            call_span,
        );
        self.generated_inference
            .callsite_candidates
            .entry(key)
            .or_default()
            .push(SemanticCallSiteCandidate {
                instantiations,
                parameter_types: Some(parameter_types),
                arguments: Some(arguments),
            });
    }
}

fn receiver_parameter_pattern(receiver: &Type, variables: &[TypeVar]) -> Option<Type> {
    match receiver {
        Type::Generic { base, args } if args.len() == variables.len() => Some(Type::Generic {
            base: base.clone(),
            args: variables.iter().cloned().map(Type::Variable).collect(),
        }),
        Type::Concrete(TypeAnnotation::Array(_)) if variables.len() == 1 => Some(Type::Concrete(
            TypeAnnotation::Array(Box::new(tyvar_to_annotation(&variables[0]))),
        )),
        Type::Concrete(TypeAnnotation::Generic { name, args }) if args.len() == variables.len() => {
            Some(Type::Concrete(TypeAnnotation::Generic {
                name: name.clone(),
                args: variables.iter().map(tyvar_to_annotation).collect(),
            }))
        }
        _ if variables.is_empty() => Some(receiver.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use shape_ast::ast::Item;
    use shape_ast::parser::parse_program;

    use super::*;
    use crate::type_system::{BuiltinTypes, SemanticCallSiteFact};

    #[test]
    fn generic_method_hof_call_publishes_receiver_and_method_arguments() {
        let program = parse_program(
            r#"
                extend Vec<T> {
                    method map<U>(f: (T) => U) -> Vec<U> {
                        [f(self[0])]
                    }
                }

                let mapped = [1, 2].map(|value| true)
            "#,
        )
        .expect("generic method fixture parses");
        let mut engine = TypeInferenceEngine::new();

        let (facts, errors) = engine.infer_program_facts_best_effort(&program);

        assert!(errors.is_empty(), "unexpected inference errors: {errors:?}");
        let fact = facts
            .semantic_callsite_facts()
            .iter()
            .find_map(|(key, fact)| (key.callee() == "Vec.map").then_some(fact))
            .expect("method call must publish qualified semantic evidence");
        let SemanticCallSiteFact::Exact(exact) = fact else {
            panic!("method call must preserve exact evidence, got {fact:?}")
        };
        let arguments = exact.arguments();
        assert_eq!(arguments.len(), 2);
        assert!(
            facts
                .semantic_callee_declaration("Vec.map")
                .expect("qualified method declaration is published")
                .matches_exact(exact),
            "qualified active declaration must match every sealed argument"
        );
        assert_eq!(arguments[0].source_name(), "T");
        assert_eq!(arguments[0].candidate().ty(), &BuiltinTypes::integer());
        assert_eq!(arguments[1].source_name(), "U");
        assert_eq!(arguments[1].candidate().ty(), &BuiltinTypes::boolean());
    }

    #[test]
    fn identical_qualified_method_declarations_keep_distinct_capabilities() {
        let program = parse_program(
            r#"
                extend Vec<T> { method map<U>(f: (T) => U) -> Vec<U> { [] } }
                extend Vec<T> { method map<U>(f: (T) => U) -> Vec<U> { [] } }
            "#,
        )
        .expect("duplicate-shape declaration fixture parses");
        let mut engine = TypeInferenceEngine::new();
        let mut declarations = Vec::new();
        for item in &program.items {
            let Item::Extend(statement, _) = item else {
                continue;
            };
            engine.register_extend(statement).expect("method registers");
            declarations.push(InferenceMethodDeclarationToken::of(&statement.methods[0]));
        }

        assert_eq!(declarations.len(), 2);
        assert_ne!(declarations[0], declarations[1]);
        assert!(
            engine
                .generated_inference
                .method_declared_parameters
                .contains_key(&declarations[0])
        );
        assert!(
            engine
                .generated_inference
                .method_declared_parameters
                .contains_key(&declarations[1])
        );
        assert_eq!(
            engine
                .generated_inference
                .active_method_declarations
                .get("Vec.map"),
            Some(&declarations[1])
        );
    }

    #[test]
    fn receiver_aliases_share_one_method_declaration_capability() {
        let program = parse_program("extend number { method convert<T>(value: T) -> T { value } }")
            .expect("numeric alias fixture parses");
        let Item::Extend(statement, _) = &program.items[0] else {
            panic!("expected extend item")
        };
        let mut engine = TypeInferenceEngine::new();

        engine.register_extend(statement).expect("method registers");
        engine.finalize_semantic_callee_declarations();

        let number = engine.generated_inference.active_method_declarations["number.convert"];
        let integer = engine.generated_inference.active_method_declarations["int.convert"];
        assert_eq!(number, integer);
        assert_eq!(
            engine.generated_inference.method_declared_parameters.len(),
            1
        );
        assert_eq!(
            engine.generated_inference.callee_declarations["number.convert"],
            engine.generated_inference.callee_declarations["int.convert"]
        );
    }

    #[test]
    fn reused_engine_mints_fresh_method_declaration_capabilities() {
        let source = "extend Vec<T> { method map<U>(f: (T) => U) -> Vec<U> { [f(self[0])] } }";
        let first = parse_program(source).expect("first program parses");
        let second = parse_program(source).expect("second program parses");
        let mut engine = TypeInferenceEngine::new();

        let _ = engine.infer_program_best_effort(&first);
        let first_parameters = engine
            .generated_inference
            .active_method_declarations
            .get("Vec.map")
            .and_then(|token| {
                engine
                    .generated_inference
                    .method_declared_parameters
                    .get(token)
            })
            .expect("first declaration parameters")
            .receiver
            .clone();
        let _ = engine.infer_program_best_effort(&second);
        let second_parameters = engine
            .generated_inference
            .active_method_declarations
            .get("Vec.map")
            .and_then(|token| {
                engine
                    .generated_inference
                    .method_declared_parameters
                    .get(token)
            })
            .expect("second declaration parameters")
            .receiver
            .clone();

        assert_ne!(first_parameters, second_parameters);
    }

    #[test]
    fn unannotated_generic_method_return_reuses_registered_capabilities() {
        let program = parse_program(
            r#"
                extend Vec<T> {
                    method retain(item: T) { item }
                }

                let value = [1].retain(2)
            "#,
        )
        .expect("unannotated return fixture parses");
        let mut engine = TypeInferenceEngine::new();

        let (_, errors) = engine.infer_program_best_effort(&program);

        assert!(errors.is_empty(), "unexpected inference errors: {errors:?}");
    }
}
