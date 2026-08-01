use super::*;

use shape_ast::ast::Item;

/// #240 (C-harness): a compiler carrying the inference facts that `compile()`
/// would have installed for `source`.
///
/// `BytecodeCompiler::new()` has an EMPTY `resolved_expr_types`. Production
/// never reaches descriptor construction in that state — `compile()` populates
/// the span table before any body compiles — so a harness that skips it leaves
/// the return classifier with nothing to classify a closure's terminal
/// expression from. A missing test precondition, not a compiler gap: the same
/// sources compile clean through `compile()`. Mirrors the idiom in
/// `compiler/functions/reference_provenance_tests.rs`.
fn compiler_with_inference_facts(source: &str) -> BytecodeCompiler {
    let program = shape_ast::parse_program(source).expect("fixture parses");
    let mut compiler = BytecodeCompiler::new();
    let facts = BytecodeCompiler::infer_reference_model(&program).3;
    compiler.resolved_expr_types = facts.expression_types().clone();
    compiler.inference_facts = facts;
    compiler
}

fn source_function(source: &str, name: &str) -> FunctionDef {
    shape_ast::parse_program(source)
        .expect("fixture parses")
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::Function(function, _) if function.name == name => Some(function),
            _ => None,
        })
        .unwrap_or_else(|| panic!("fixture must define function '{name}'"))
}

fn annotation_carrier_identity(compiler: &BytecodeCompiler, name: &str) -> Option<String> {
    compiler
        .program
        .compiled_annotations
        .get(name)
        .map(|carrier| format!("{carrier:#?}"))
}

fn compile_annotated_function(
    source: &str,
    name: &str,
) -> (BytecodeCompiler, std::result::Result<(), String>) {
    let program = shape_ast::parse_program(source).expect("fixture parses");
    let function = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(function, _) if function.name == name => Some(function.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("fixture must define function '{name}'"));
    let annotation_names = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::AnnotationDef(definition, _) => Some(definition.name.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [expected_annotation_name] = annotation_names.as_slice() else {
        panic!("fixture must define exactly one annotation")
    };
    // #240 (C-harness): install the span table `compile()` would have built
    // before any body compiles. Without it a closure in the target body (or in
    // a `replace body` directive) reaches the descriptor builder with no
    // resolvable terminal type — a missing precondition of this harness, not a
    // compiler gap.
    let mut compiler = compiler_with_inference_facts(source);
    compiler
        .register_function(&function)
        .expect("target function registers");
    compiler
        .install_semantic_freeze()
        .expect("registration-complete fixture freezes");
    compiler
        .prepare_annotation_scope(&program.items)
        .expect("annotation scope prepares before pass two");
    let prepared_cardinality = compiler.program.compiled_annotations.len();
    let prepared_identity = annotation_carrier_identity(&compiler, expected_annotation_name)
        .expect("expected annotation carrier is installed during preparation");
    for item in &program.items {
        if matches!(item, Item::AnnotationDef(..)) {
            compiler
                .compile_item_with_context(item, false)
                .expect("annotation definition consumes preparation evidence");
        }
    }
    assert_eq!(
        compiler.program.compiled_annotations.len(),
        prepared_cardinality
    );
    assert_eq!(
        annotation_carrier_identity(&compiler, expected_annotation_name)
            .expect("pass two retains the prepared annotation carrier"),
        prepared_identity,
        "pass two must not reinstall the prepared annotation carrier"
    );
    let outcome = compiler
        .compile_function(&function)
        .map_err(|error| error.to_string());
    (compiler, outcome)
}

fn install_frozen_owners(compiler: &mut BytecodeCompiler, owners: &[&FunctionDef]) {
    for owner in owners {
        compiler
            .register_function(owner)
            .expect("semantic owner registers");
    }
    compiler
        .install_semantic_freeze()
        .expect("registration-complete fixture freezes");
}

fn authenticated_capability(
    compiler: &BytecodeCompiler,
    owner: &FunctionDef,
) -> OriginalCapability {
    compiler
        .build_original_capability(owner, compiler.original_body_shadow_name(&owner.name))
        .expect("semantic freeze issues an original-body capability")
}

fn artifact_counts(
    compiler: &BytecodeCompiler,
) -> (usize, usize, usize, usize, usize, usize, usize) {
    (
        compiler.function_defs.len(),
        compiler.program.functions.len(),
        compiler.mir_functions.len(),
        compiler.mir_storage_plans.len(),
        compiler.closure_capture_packs.len(),
        compiler.closure_type_ids.len(),
        compiler.function_type_ids.len(),
    )
}

fn assert_prepublication_refusal(
    compiler: &BytecodeCompiler,
    shadow_name: &str,
    before: (usize, usize, usize, usize, usize, usize, usize),
) {
    assert_eq!(artifact_counts(compiler), before);
    assert!(!compiler.function_defs.contains_key(shadow_name));
    assert!(compiler.find_function(shadow_name).is_none());
    assert!(!compiler.mir_functions.contains_key(shadow_name));
    assert!(!compiler.mir_storage_plans.contains_key(shadow_name));
}

fn assert_exact_invariant(error: ShapeError, expected: &str) {
    match error {
        ShapeError::RuntimeError { message, location } => {
            assert_eq!(message, expected);
            assert!(location.is_none());
        }
        other => panic!("expected RuntimeError capability invariant, got {other:?}"),
    }
}

#[test]
fn authentic_capability_has_a_complete_callable_payload_and_publishes() {
    let owner = source_function("fn probe(value: &int) -> int { 1 }", "probe");
    assert!(owner.params[0].is_reference);
    assert!(!owner.params[0].is_mut_reference);
    assert!(matches!(
        owner.params[0].type_annotation.as_ref(),
        Some(TypeAnnotation::Basic(name)) if name == "int"
    ));
    let mut compiler = BytecodeCompiler::new();
    install_frozen_owners(&mut compiler, &[&owner]);
    let capability = authenticated_capability(&compiler, &owner);
    let shadow_name = capability.shadow_name().to_string();
    let freeze = compiler
        .comptime_freeze_overlay()
        .expect("installed semantic freeze");
    let owner_callable = canonical_original_callable(freeze.as_ref(), &owner)
        .expect("parser-authoritative shared-borrow signature canonicalizes");
    assert_eq!(capability.callable(), owner_callable);
    let mut exclusive_owner = owner.clone();
    exclusive_owner.params[0].is_mut_reference = true;
    let exclusive_callable = canonical_original_callable(freeze.as_ref(), &exclusive_owner)
        .expect("mode-only exclusive-borrow signature canonicalizes");
    assert_ne!(
        owner_callable, exclusive_callable,
        "shared and exclusive passing modes must have distinct callable identities"
    );
    let payload = freeze
        .payload_of(owner_callable)
        .expect("stored callable has a complete payload");
    assert_eq!(payload.category(), FrozenTypeCategory::Callable);

    let pending =
        PendingOriginalBodyShadow::new(&owner, capability, &[None], &[ParamPassMode::ByRefShared])
            .expect("authenticated pending shadow");
    compiler
        .finalize_pending_original_body_shadow(pending)
        .expect("authenticated shadow publishes");
    assert!(compiler.function_defs.contains_key(&shadow_name));
    assert!(compiler.find_function(&shadow_name).is_some());
}

#[test]
fn staged_emission_passing_mode_tamper_refuses_before_publication() {
    let owner = source_function("fn probe(value: &int) -> int { value }", "probe");
    assert!(owner.params[0].is_reference);
    assert!(!owner.params[0].is_mut_reference);
    let mut compiler = BytecodeCompiler::new();
    install_frozen_owners(&mut compiler, &[&owner]);
    let capability = authenticated_capability(&compiler, &owner);
    let shadow_name = capability.shadow_name().to_string();
    let mut pending =
        PendingOriginalBodyShadow::new(&owner, capability, &[None], &[ParamPassMode::ByRefShared])
            .expect("authenticated pending shadow");
    pending.emission.params[0].is_mut_reference = true;
    let before = artifact_counts(&compiler);

    let error = compiler
        .finalize_pending_original_body_shadow(pending)
        .expect_err("mode-only signature tamper must refuse");

    assert_exact_invariant(error, CALLABLE_IDENTITY_DIAGNOSTIC);
    assert_prepublication_refusal(&compiler, &shadow_name, before);
}

#[test]
fn same_signature_foreign_owner_capability_refuses_before_publication() {
    let owner = source_function("fn owner(value: int) -> int { value }", "owner");
    let foreign = source_function("fn foreign(value: int) -> int { value }", "foreign");
    let mut compiler = BytecodeCompiler::new();
    install_frozen_owners(&mut compiler, &[&owner, &foreign]);
    let freeze = compiler
        .comptime_freeze_overlay()
        .expect("installed semantic freeze");
    assert_eq!(
        canonical_original_callable(freeze.as_ref(), &owner),
        canonical_original_callable(freeze.as_ref(), &foreign),
        "the refusal must not rely on differing callable signatures"
    );
    let capability = authenticated_capability(&compiler, &foreign);
    let shadow_name = capability.shadow_name().to_string();
    let pending =
        PendingOriginalBodyShadow::new(&owner, capability, &[None], &[ParamPassMode::ByValue])
            .expect("cardinality-valid foreign binding reaches final validation");
    let before = artifact_counts(&compiler);

    let error = compiler
        .finalize_pending_original_body_shadow(pending)
        .expect_err("foreign owner binding must refuse");

    assert_exact_invariant(error, SHADOW_IDENTITY_DIAGNOSTIC);
    assert_prepublication_refusal(&compiler, &shadow_name, before);
}

#[test]
fn staged_emission_signature_tamper_refuses_before_publication() {
    let owner = source_function("fn probe(value: int) -> int { value }", "probe");
    let mut compiler = BytecodeCompiler::new();
    install_frozen_owners(&mut compiler, &[&owner]);
    let capability = authenticated_capability(&compiler, &owner);
    let shadow_name = capability.shadow_name().to_string();
    let mut pending =
        PendingOriginalBodyShadow::new(&owner, capability, &[None], &[ParamPassMode::ByValue])
            .expect("authenticated pending shadow");
    pending.emission.return_type = Some(TypeAnnotation::Basic("string".to_string()));
    let before = artifact_counts(&compiler);

    let error = compiler
        .finalize_pending_original_body_shadow(pending)
        .expect_err("signature tamper must refuse");

    assert_exact_invariant(error, CALLABLE_IDENTITY_DIAGNOSTIC);
    assert_prepublication_refusal(&compiler, &shadow_name, before);
}

#[test]
fn invalid_or_non_callable_identity_refuses_before_payload_and_publication() {
    let owner = source_function("fn probe(value: int) -> int { value }", "probe");
    let mut compiler = BytecodeCompiler::new();
    install_frozen_owners(&mut compiler, &[&owner]);
    let capability = authenticated_capability(&compiler, &owner);
    let shadow_name = capability.shadow_name().to_string();
    let pending =
        PendingOriginalBodyShadow::new(&owner, capability, &[None], &[ParamPassMode::ByValue])
            .expect("authenticated pending shadow");
    let freeze = compiler
        .comptime_freeze_overlay()
        .expect("installed semantic freeze");
    let int_identity = freeze
        .identity_of("int")
        .expect("frozen primitive identity");

    for invalid in [int_identity, FrozenTypeIdentity::INVALID] {
        let mut tampered = pending.clone();
        tampered.capability.callable = invalid;
        let before = artifact_counts(&compiler);
        let error = compiler
            .finalize_pending_original_body_shadow(tampered)
            .expect_err("non-callable stored identity must refuse");
        assert_exact_invariant(error, CALLABLE_IDENTITY_DIAGNOSTIC);
        assert_prepublication_refusal(&compiler, &shadow_name, before);
    }
}

#[test]
fn remove_target_discards_a_staged_original_body_shadow() {
    let source = r#"
annotation replace_then_remove() on function {
  comptime post(target, ctx) {
    replace body { return ctx.original(value) }
    remove target
  }
}

@replace_then_remove()
fn probe(value: int) -> int { value + 1 }
"#;
    let identity_compiler = BytecodeCompiler::new();
    let shadow = identity_compiler.original_body_shadow_name("probe");
    let (compiler, outcome) = compile_annotated_function(source, "probe");

    outcome.expect("remove target completes without publishing the staged shadow");
    assert!(compiler.removed_functions.contains("probe"));
    assert!(!compiler.function_defs.contains_key(&shadow));
    assert!(compiler.find_function(&shadow).is_none());
    assert!(!compiler.mir_functions.contains_key(&shadow));
    assert!(!compiler.mir_storage_plans.contains_key(&shadow));
}

#[test]
fn repeated_replace_body_is_rejected_before_shadow_publication() {
    let source = r#"
annotation replace_twice() on function {
  comptime post(target, ctx) {
    replace body { return ctx.original(value) }
    replace body { return value }
  }
}

@replace_twice()
fn probe(value: int) -> int { value + 1 }
"#;
    let identity_compiler = BytecodeCompiler::new();
    let shadow = identity_compiler.original_body_shadow_name("probe");
    let (compiler, outcome) = compile_annotated_function(source, "probe");

    let error = outcome.expect_err("a second replacement is ambiguous");
    assert!(
        error.contains("multiple `replace body` directives for function 'probe' are ambiguous"),
        "unexpected diagnostic: {error}"
    );
    assert!(!compiler.function_defs.contains_key(&shadow));
    assert!(compiler.find_function(&shadow).is_none());
    assert!(!compiler.mir_functions.contains_key(&shadow));
    assert!(!compiler.mir_storage_plans.contains_key(&shadow));
}

#[test]
fn replacement_mir_uses_only_its_own_distinct_closure_identity() {
    let source = r#"
annotation replace_with_closure() on function {
  comptime post(target, ctx) {
    replace body {
      let replacement = |left: int, right: int; | left + right
      return replacement(value, 100)
    }
  }
}

@replace_with_closure()
fn probe(value: int) -> int {
  let unary = |item: int| item + 1
  unary(value)
}
"#;
    let (compiler, outcome) = compile_annotated_function(source, "probe");
    outcome.expect("distinct original and replacement closures compile");

    let closures: Vec<_> = compiler
        .program
        .functions
        .iter()
        .filter(|function| function.is_closure)
        .collect();
    assert_eq!(
        closures.len(),
        2,
        "both persistent closure artifacts remain"
    );
    let shadow_closure = closures
        .iter()
        .find(|function| function.arity == 1)
        .expect("the original shadow owns the unary closure");
    let replacement_closure = closures
        .iter()
        .find(|function| function.arity == 2)
        .expect("the replacement owns the binary closure");
    assert_ne!(shadow_closure.name, replacement_closure.name);

    let replacement_mir = compiler
        .program
        .functions
        .iter()
        .find(|function| function.name == "probe")
        .and_then(|function| function.mir_data.as_ref())
        .expect("public replacement publishes MIR data");
    let exact_closure_identities = [
        shadow_closure.name.as_str(),
        replacement_closure.name.as_str(),
    ];
    let referenced_closures: Vec<_> = replacement_mir
        .mir
        .iter_blocks()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            crate::mir::types::StatementKind::Assign(
                _,
                crate::mir::types::Rvalue::Use(crate::mir::types::Operand::Constant(
                    crate::mir::types::MirConstant::Function(name),
                )),
            ) if exact_closure_identities.contains(&name.as_str()) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        referenced_closures,
        vec![replacement_closure.name.as_str()],
        "replacement MIR must never consume the shadow closure's transient backpatch identity"
    );
}

#[test]
fn failed_shadow_emission_restores_body_analysis_authority() {
    const SOURCE: &str = "fn probe(value: int) -> int { let worker = |item: int| item + 1\nlet observed = worker(value)\nmissing_value }";
    let semantic_owner = source_function(SOURCE, "probe");
    // #240 (C-harness): the body contains a closure, so the return classifier
    // needs the span table `compile()` would have installed.
    let mut compiler = compiler_with_inference_facts(SOURCE);
    install_frozen_owners(&mut compiler, &[&semantic_owner]);
    let capability = authenticated_capability(&compiler, &semantic_owner);
    let shadow_name = capability.shadow_name().to_string();
    let pending = PendingOriginalBodyShadow::new(
        &semantic_owner,
        capability,
        &[None],
        &[ParamPassMode::ByValue],
    )
    .expect("slot-aligned authenticated pending shadow");
    let outer_closure_ids = vec![("outer-closure".to_string(), 73)];
    compiler.closure_function_ids = outer_closure_ids.clone();

    let error = compiler
        .finalize_pending_original_body_shadow(pending)
        .expect_err("undefined source identifier must fail during shadow emission");

    assert!(
        error.to_string().contains("missing_value"),
        "unexpected emission error: {error}"
    );
    assert!(compiler.active_body_analysis_authority.is_none());
    assert_eq!(compiler.closure_function_ids, outer_closure_ids);
    assert!(compiler.mir_storage_plans.contains_key("probe"));
    assert!(!compiler.mir_storage_plans.contains_key(&shadow_name));
    assert_eq!(
        (
            compiler
                .program
                .functions
                .iter()
                .filter(|function| function.is_closure)
                .count(),
            compiler.closure_capture_packs.len(),
            compiler.closure_type_ids.len(),
            compiler.function_type_ids.len(),
        ),
        (1, 1, 1, 1),
        "persistent rejected-shadow artifacts follow the existing quarantine convention"
    );
    assert!(
        compiler
            .program
            .functions
            .iter()
            .find(|function| function.name == shadow_name)
            .is_some_and(|function| function.entry_point > 0),
        "the error must occur after entering the registered shadow body"
    );
}

#[test]
fn pending_shadow_rejects_misaligned_reference_provenance() {
    let semantic_owner = source_function("fn probe(value: int) -> int { value }", "probe");
    let mut compiler = BytecodeCompiler::new();
    install_frozen_owners(&mut compiler, &[&semantic_owner]);
    let capability = authenticated_capability(&compiler, &semantic_owner);

    let error =
        PendingOriginalBodyShadow::new(&semantic_owner, capability, &[], &[ParamPassMode::ByValue])
            .expect_err("missing provenance is a structural error");

    assert!(
        error.to_string().contains(
            "function 'probe' has 1 parameters but 0 inferred-reference provenance entries"
        )
    );
}
