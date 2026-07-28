use super::*;
use crate::mir::{SlotId, StoragePlan};
use crate::type_tracking::BindingStorageClass;
use shape_ast::ast::{FunctionDef, Item};
use shape_ast::error::{Result, ShapeError};
use shape_value::v2::ConcreteType;
use std::collections::{HashMap, HashSet};

fn parsed_function(source: &str) -> FunctionDef {
    shape_ast::parser::parse_program(source)
        .expect("parse authority fixture")
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::Function(definition, _) => Some(definition),
            _ => None,
        })
        .expect("fixture must contain a function")
}

fn identical_emission_pair() -> (FunctionDef, FunctionDef) {
    let source = parsed_function("fn semantic_owner(value) { value }");
    let mut emission = source.clone();
    emission.name = "hygienic_impl".to_string();
    emission.annotations.clear();
    emission.doc_comment = None;
    (source, emission)
}

fn register(compiler: &mut BytecodeCompiler, definition: &FunctionDef) -> usize {
    compiler
        .register_function(definition)
        .expect("register authority fixture");
    compiler
        .find_function(&definition.name)
        .expect("registered function id")
}

#[test]
fn authority_applies_only_to_exact_emission_id() {
    let (source, emission) = identical_emission_pair();
    let nested = parsed_function("fn nested(value) { value }");
    let mut compiler = BytecodeCompiler::new();
    register(&mut compiler, &source);
    let emission_id = register(&mut compiler, &emission);
    let nested_id = register(&mut compiler, &nested);
    compiler.current_function = Some(emission_id);

    compiler
        .with_body_analysis_authority(emission_id, &source, &emission, |compiler| {
            assert_eq!(
                compiler.current_body_semantic_owner_key(),
                Some(source.name.as_str())
            );
            compiler.current_function = Some(nested_id);
            assert_eq!(
                compiler.current_body_semantic_owner_key(),
                Some(nested.name.as_str())
            );
            compiler.current_function = Some(emission_id);
            Ok(())
        })
        .expect("authority scope");
    assert!(compiler.active_body_analysis_authority.is_none());
}

#[test]
fn structural_mismatch_refuses_authority_before_running_body() {
    let (source, mut emission) = identical_emission_pair();
    emission.is_async = true;
    let mut compiler = BytecodeCompiler::new();
    let emission_id = register(&mut compiler, &emission);
    let mut called = false;

    let result = compiler.with_body_analysis_authority(emission_id, &source, &emission, |_| {
        called = true;
        Ok(())
    });

    let ShapeError::RuntimeError { message, .. } = result.expect_err("mismatch must fail") else {
        panic!("mismatch must be an internal invariant error");
    };
    assert!(message.contains("async marker"), "{message}");
    assert!(!called);
    assert!(compiler.active_body_analysis_authority.is_none());
}

#[test]
fn forced_error_restores_enclosing_authority_scope() {
    let (outer_source, outer_emission) = identical_emission_pair();
    let mut inner_source = outer_source.clone();
    inner_source.name = "inner_source".to_string();
    let mut inner_emission = inner_source.clone();
    inner_emission.name = "inner_impl".to_string();
    let mut compiler = BytecodeCompiler::new();
    let outer_id = register(&mut compiler, &outer_emission);
    let inner_id = register(&mut compiler, &inner_emission);
    compiler.current_function = Some(outer_id);

    compiler
        .with_body_analysis_authority(outer_id, &outer_source, &outer_emission, |compiler| {
            let enclosing = compiler.active_body_analysis_authority.clone();
            let inner_result: Result<()> = compiler.with_body_analysis_authority(
                inner_id,
                &inner_source,
                &inner_emission,
                |_| {
                    Err(ShapeError::RuntimeError {
                        message: "forced authority failure".to_string(),
                        location: None,
                    })
                },
            );
            assert!(inner_result.is_err());
            assert_eq!(compiler.active_body_analysis_authority, enclosing);
            Ok(())
        })
        .expect("outer scope");
    assert!(compiler.active_body_analysis_authority.is_none());
}

#[test]
fn wrapper_identity_never_observes_impl_authority() {
    let (source, emission) = identical_emission_pair();
    let wrapper = parsed_function("fn wrapper(value) { value }");
    let mut compiler = BytecodeCompiler::new();
    let emission_id = register(&mut compiler, &emission);
    let wrapper_id = register(&mut compiler, &wrapper);
    compiler.current_function = Some(emission_id);

    compiler
        .with_body_analysis_authority(emission_id, &source, &emission, |compiler| {
            compiler.current_function = Some(wrapper_id);
            assert_eq!(
                compiler.current_body_semantic_owner_key(),
                Some(wrapper.name.as_str())
            );
            Ok(())
        })
        .expect("authority scope");
}

#[test]
fn authority_reads_source_plan_without_hygienic_map_aliases() {
    let (source, emission) = identical_emission_pair();
    let mut compiler = BytecodeCompiler::new();
    let emission_id = register(&mut compiler, &emission);
    compiler.mir_storage_plans.insert(
        source.name.clone(),
        StoragePlan {
            slot_classes: HashMap::from([(SlotId(1), BindingStorageClass::Direct)]),
            slot_semantics: HashMap::new(),
            inline_array_sizes: HashMap::new(),
            non_escaping_closure_slots: HashSet::new(),
            reference_escape_promotion_slots: HashSet::new(),
        },
    );
    compiler.current_function = Some(emission_id);

    compiler
        .with_body_analysis_authority(emission_id, &source, &emission, |compiler| {
            assert_eq!(
                compiler
                    .current_storage_plan()
                    .and_then(|plan| plan.slot_classes.get(&SlotId(1))),
                Some(&BindingStorageClass::Direct)
            );
            assert!(!compiler.mir_storage_plans.contains_key(&emission.name));
            assert!(!compiler.mir_borrow_analyses.contains_key(&emission.name));
            assert!(!compiler.mir_span_to_point.contains_key(&emission.name));
            Ok(())
        })
        .expect("authority scope");

    assert!(!compiler.mir_storage_plans.contains_key(&emission.name));
}

#[test]
fn exclusive_impl_metadata_and_return_projection_are_explicit() {
    let (source, emission) = identical_emission_pair();
    let mut compiler = BytecodeCompiler::new();
    let emission_id = register(&mut compiler, &emission);
    compiler
        .type_tracker
        .register_function_return_concrete_type(&source.name, ConcreteType::I64);

    compiler
        .refresh_authoritative_emission_metadata(
            emission_id,
            &emission,
            &source.name,
            &[ParamPassMode::ByRefExclusive],
        )
        .expect("refresh exact impl metadata");

    let registered = &compiler.program.functions[emission_id];
    assert_eq!(registered.ref_params, vec![true]);
    assert_eq!(registered.ref_mutates, vec![true]);
    assert!(
        !compiler
            .inferred_param_pass_modes
            .contains_key(&emission.name)
    );
    assert_eq!(
        compiler
            .type_tracker
            .get_function_return_concrete_type(&emission.name),
        Some(&ConcreteType::I64)
    );
}
