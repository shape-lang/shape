//! Acceptance evidence for #91 (R16, R17, ADR-013 §7).
//!
//! The tracer is fixed by R17 and is not substituted: `fn add(a: int, b: int)
//! -> int` plus one call site. Every test below inspects the *resolved fact* —
//! identity, normalized contract, structured diagnostics — never a rendered
//! string, and every invalidation claim is measured from Salsa's own event
//! stream rather than assumed.
//!
//! The six edit traces required by R16 are `comment_only_edit_*`,
//! `body_only_edit_*`, `signature_edit_*`, `import_retarget_edit_*`,
//! `alias_edit_*` and `local_shadow_edit_*`. Each states its expectation in
//! prose first and then asserts it.

use crate::diagnostics::codes;
use crate::identity::DefinitionPath;
use crate::trace::QueryTrace;
use crate::{NormalizedType, SemanticSession};

const MATH_UNIT: &str = "app::math";
const MATH2_UNIT: &str = "app::math2";
const MAIN_UNIT: &str = "app::main";

const MATH_SOURCE: &str = "pub fn add(a: int, b: int) -> int {\n    a + b\n}\n";
const MATH2_SOURCE: &str = "pub fn add(a: int, b: int) -> int {\n    a - b\n}\n";
const MAIN_SOURCE: &str = "from app::math use { add }\n\nlet total = add(1, 2)\n";

/// The tracer program. `app::math2` declares a same-named callable used only by
/// the import-retarget trace; it is present from the start so that trace edits
/// text and nothing else.
fn tracer_session(trace: bool) -> SemanticSession {
    let mut session = if trace {
        SemanticSession::with_query_trace()
    } else {
        SemanticSession::new()
    };
    session.insert_unit(MATH_UNIT, MATH_SOURCE);
    session.insert_unit(MATH2_UNIT, MATH2_SOURCE);
    session.insert_unit(MAIN_UNIT, MAIN_SOURCE);
    session
}

/// Demands every published fact so the memo graph is complete.
fn demand(session: &SemanticSession) {
    session.callable_facts_of(MAIN_UNIT, "add");
    session.call_site_facts(MAIN_UNIT, 0);
}

/// Settles the session and discards the resulting trace, leaving a clean window
/// in which to measure one edit.
fn settle(session: &SemanticSession) -> QueryTrace {
    demand(session);
    session.take_trace()
}

fn math_add_identity() -> crate::DefinitionIdentity {
    DefinitionPath::top_level_callable(MATH_UNIT, "add", 0).identity()
}

// ---------------------------------------------------------------------------
// The published facts themselves
// ---------------------------------------------------------------------------

#[test]
fn publishes_identity_contract_diagnostics_and_provenance_for_the_tracer() {
    let session = tracer_session(false);
    let facts = session
        .callable_facts_of(MAIN_UNIT, "add")
        .expect("the tracer resolves");

    // Resolved definition identity, issued by the database and equal to the
    // identity computed from canonical data with no database at all.
    assert_eq!(facts.identity(), math_add_identity());

    // Normalized base contract, inspected structurally.
    let contract = facts.contract();
    assert_eq!(contract.params.len(), 2);
    assert_eq!(contract.params[0].name, "a");
    assert_eq!(contract.params[0].ty, NormalizedType::Int);
    assert_eq!(contract.params[1].name, "b");
    assert_eq!(contract.params[1].ty, NormalizedType::Int);
    assert_eq!(contract.result, NormalizedType::Int);
    assert!(!contract.is_async);
    assert!(!contract.is_comptime);
    assert!(contract.type_params.is_empty());

    // Deterministic diagnostics: a fully declared contract has none.
    assert!(
        facts.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        facts.diagnostics
    );

    // Source provenance points at the declaring unit and the declaration.
    assert_eq!(facts.provenance.unit_path, MATH_UNIT);
    assert_eq!(
        facts.provenance.unit_identity,
        crate::UnitIdentity::for_path(MATH_UNIT)
    );
    let name_span = facts.provenance.name_span;
    assert_eq!(&MATH_SOURCE[name_span.start..name_span.end], "add");
}

#[test]
fn the_call_site_names_the_definition_the_declaration_published() {
    let session = tracer_session(false);
    let facts = session.callable_facts_of(MAIN_UNIT, "add").unwrap();
    let call = session.call_site_facts(MAIN_UNIT, 0).unwrap();

    assert_eq!(call.callee_identity(), Some(facts.identity()));
    assert_eq!(call.written_name, "add");
    assert_eq!(
        call.argument_types,
        vec![Some(NormalizedType::Int), Some(NormalizedType::Int)]
    );
    // The call site was checked against the published contract, not against a
    // second copy of the signature.
    assert_eq!(
        call.callee_contract_identity,
        Some(
            session
                .contract_facts_of(MAIN_UNIT, "add")
                .unwrap()
                .content_identity()
        )
    );
    assert!(call.diagnostics.is_empty(), "{:?}", call.diagnostics);
}

#[test]
fn identity_keyed_entry_point_returns_the_same_facts() {
    let session = tracer_session(false);
    let by_name = session.callable_facts_of(MAIN_UNIT, "add").unwrap();
    let by_identity = session
        .callable_facts(by_name.identity())
        .expect("identity-keyed lookup finds the definition");
    assert_eq!(by_name.content_identity(), by_identity.content_identity());
}

// ---------------------------------------------------------------------------
// ADR-013 §7.1 — fact identity across independently created sessions
// ---------------------------------------------------------------------------

#[test]
fn fresh_sessions_agree_on_fact_content_identity() {
    let compiler_role = tracer_session(false);
    let tooling_role = tracer_session(false);

    let compiler_facts = compiler_role.callable_facts_of(MAIN_UNIT, "add").unwrap();
    let tooling_facts = tooling_role.callable_facts_of(MAIN_UNIT, "add").unwrap();
    assert_eq!(compiler_facts.identity(), tooling_facts.identity());
    assert_eq!(
        compiler_facts.content_identity(),
        tooling_facts.content_identity()
    );

    let compiler_contract = compiler_role.contract_facts_of(MAIN_UNIT, "add").unwrap();
    let tooling_contract = tooling_role.contract_facts_of(MAIN_UNIT, "add").unwrap();
    assert_eq!(
        compiler_contract.content_identity(),
        tooling_contract.content_identity()
    );

    let compiler_call = compiler_role.call_site_facts(MAIN_UNIT, 0).unwrap();
    let tooling_call = tooling_role.call_site_facts(MAIN_UNIT, 0).unwrap();
    assert_eq!(
        compiler_call.content_identity(),
        tooling_call.content_identity()
    );
}

#[test]
fn database_local_ids_do_not_reach_published_identities() {
    // Inserting the units in a different order gives every unit a different
    // Salsa id. If any local handle leaked into a published identity, these
    // digests would differ.
    let mut forward = SemanticSession::new();
    forward.insert_unit(MATH_UNIT, MATH_SOURCE);
    forward.insert_unit(MATH2_UNIT, MATH2_SOURCE);
    forward.insert_unit(MAIN_UNIT, MAIN_SOURCE);

    let mut reverse = SemanticSession::new();
    reverse.insert_unit(MAIN_UNIT, MAIN_SOURCE);
    reverse.insert_unit(MATH2_UNIT, MATH2_SOURCE);
    reverse.insert_unit(MATH_UNIT, MATH_SOURCE);

    let forward_facts = forward.callable_facts_of(MAIN_UNIT, "add").unwrap();
    let reverse_facts = reverse.callable_facts_of(MAIN_UNIT, "add").unwrap();
    assert_eq!(forward_facts.identity(), reverse_facts.identity());
    assert_eq!(
        forward_facts.content_identity(),
        reverse_facts.content_identity()
    );
    assert_eq!(
        forward.call_site_facts(MAIN_UNIT, 0).unwrap(),
        reverse.call_site_facts(MAIN_UNIT, 0).unwrap()
    );
}

// ---------------------------------------------------------------------------
// R17 — positive alias, negative homonym
// ---------------------------------------------------------------------------

#[test]
fn an_import_alias_resolves_to_the_same_definition_identity() {
    let mut aliased = tracer_session(false);
    aliased.set_unit_text(
        MAIN_UNIT,
        "from app::math use { add as plus }\n\nlet total = plus(1, 2)\n",
    );

    let facts = aliased
        .callable_facts_of(MAIN_UNIT, "plus")
        .expect("the alias resolves");
    assert_eq!(facts.identity(), math_add_identity());
    // The alias is presentation: the published declaration keeps its own name.
    assert_eq!(facts.name(), "add");
    assert_eq!(
        aliased
            .call_site_facts(MAIN_UNIT, 0)
            .unwrap()
            .callee_identity(),
        Some(math_add_identity())
    );
}

#[test]
fn a_same_spelled_local_declaration_receives_no_privileged_semantics() {
    let mut shadowed = tracer_session(false);
    shadowed.set_unit_text(
        MAIN_UNIT,
        "from app::math use { add }\n\nlet total = add(1, 2)\n\nfn add(a: string, b: string) -> string {\n    a\n}\n",
    );

    let resolution = shadowed.resolve_callable(MAIN_UNIT, "add").unwrap();
    let resolved = resolution
        .resolved()
        .expect("resolves to the local homonym");
    assert_eq!(
        resolved.identity,
        DefinitionPath::top_level_callable(MAIN_UNIT, "add", 0).identity()
    );
    assert_ne!(resolved.identity, math_add_identity());
    assert_eq!(resolved.declaring_unit, MAIN_UNIT);
    assert!(
        resolution
            .diagnostics
            .iter()
            .any(|d| d.code == codes::IMPORT_SHADOWED_BY_LOCAL_DECLARATION),
        "shadowing is reported: {:?}",
        resolution.diagnostics
    );

    // Observable downstream: the call site is now checked against the homonym's
    // contract and both int arguments are rejected.
    let call = shadowed.call_site_facts(MAIN_UNIT, 0).unwrap();
    let mismatches: Vec<&crate::SemanticDiagnostic> = call
        .diagnostics
        .iter()
        .filter(|d| d.code == codes::CALL_ARGUMENT_TYPE_MISMATCH)
        .collect();
    assert_eq!(mismatches.len(), 2, "{:?}", call.diagnostics);
    assert_eq!(mismatches[0].arg("expected"), Some("string"));
    assert_eq!(mismatches[0].arg("actual"), Some("int"));
}

#[test]
fn renaming_the_call_site_spelling_does_not_change_the_definition_fact() {
    // Same declaration, two different use spellings: the published callable
    // fact is byte-identical.
    let plain = tracer_session(false);
    let mut aliased = tracer_session(false);
    aliased.set_unit_text(
        MAIN_UNIT,
        "from app::math use { add as plus }\n\nlet total = plus(1, 2)\n",
    );

    assert_eq!(
        plain
            .callable_facts_of(MAIN_UNIT, "add")
            .unwrap()
            .content_identity(),
        aliased
            .callable_facts_of(MAIN_UNIT, "plus")
            .unwrap()
            .content_identity()
    );
}

// ---------------------------------------------------------------------------
// The trace instrument itself
// ---------------------------------------------------------------------------

/// Without this, an edit trace asserting "0 executions" could be passing
/// because the matcher never matches anything. Here every query in the graph
/// executes exactly once on a cold session, and unit-keyed queries name their
/// unit.
#[test]
fn trace_events_name_the_query_and_the_unit() {
    let session = tracer_session(true);
    let trace = settle(&session);
    for expected in [
        "parsed_unit(app::math)",
        "declaration_index(app::math)",
        "parsed_unit(app::main)",
        "declaration_index(app::main)",
        "unit_provenance(app::math)",
    ] {
        assert_eq!(
            trace.executions(expected),
            1,
            "expected one execution of {expected} in {trace:#?}"
        );
    }
    for expected in [
        "resolve_callable",
        "callable_contract",
        "callable_facts",
        "call_site_facts",
        "unit_for_path",
    ] {
        assert_eq!(trace.executions(expected), 1, "{trace:#?}");
    }
    // The unit nothing referenced was never parsed.
    assert_eq!(trace.executions("declaration_index(app::math2)"), 0);
    assert_eq!(trace.executions("parsed_unit(app::math2)"), 0);
}

// ---------------------------------------------------------------------------
// R16 edit traces — declared expectation, then measured
// ---------------------------------------------------------------------------

/// **Trace 1a — comment-only edit, no span shift.**
///
/// A comment appended after the tracer changes the text of `app::math` only.
/// Declared: `parsed_unit` re-executes (it cannot backdate, the AST has no
/// structural equality); `declaration_index` and `unit_provenance` re-execute
/// and backdate; nothing semantic re-executes — no contract, no callable fact,
/// no call-site check — and `app::main` is not touched at all.
#[test]
fn comment_only_edit_after_the_declaration_cuts_off_at_the_index() {
    let mut session = tracer_session(true);
    settle(&session);
    let before = session.callable_facts_of(MAIN_UNIT, "add").unwrap();

    session.set_unit_text(
        MATH_UNIT,
        &format!("{MATH_SOURCE}\n// a comment after the declaration\n"),
    );
    demand(&session);
    let trace = session.take_trace();

    assert_eq!(trace.executions("parsed_unit(app::math)"), 1, "{trace:#?}");
    assert_eq!(trace.executions("declaration_index(app::math)"), 1);
    assert_eq!(trace.executions("unit_provenance(app::math)"), 1);
    assert_eq!(trace.executions("callable_contract"), 0);
    assert_eq!(trace.executions("callable_facts"), 0);
    assert_eq!(trace.executions("call_site_facts"), 0);
    assert_eq!(trace.executions("resolve_callable"), 0);
    assert_eq!(trace.executions("parsed_unit(app::main)"), 0);

    let after = session.callable_facts_of(MAIN_UNIT, "add").unwrap();
    assert_eq!(before.content_identity(), after.content_identity());
}

/// **Trace 1b — comment-only edit that shifts spans.**
///
/// A comment inserted *above* the tracer moves every following byte. Declared:
/// the span-free layers still backdate, so no contract re-executes and the call
/// site is not re-checked; the callable fact does re-execute, because source
/// provenance is part of what it publishes and the declaration genuinely moved.
#[test]
fn comment_only_edit_before_the_declaration_moves_provenance_only() {
    let mut session = tracer_session(true);
    settle(&session);
    let before_contract = session.contract_facts_of(MAIN_UNIT, "add").unwrap();
    let before_call = session.call_site_facts(MAIN_UNIT, 0).unwrap();
    let before_facts = session.callable_facts_of(MAIN_UNIT, "add").unwrap();
    session.take_trace();

    session.set_unit_text(MATH_UNIT, &format!("// a leading comment\n{MATH_SOURCE}"));
    demand(&session);
    let trace = session.take_trace();

    assert_eq!(trace.executions("declaration_index(app::math)"), 1);
    assert_eq!(trace.executions("unit_provenance(app::math)"), 1);
    assert_eq!(trace.executions("callable_contract"), 0, "{trace:#?}");
    assert_eq!(trace.executions("callable_facts"), 1);
    assert_eq!(trace.executions("call_site_facts"), 0);

    let after_contract = session.contract_facts_of(MAIN_UNIT, "add").unwrap();
    let after_call = session.call_site_facts(MAIN_UNIT, 0).unwrap();
    let after_facts = session.callable_facts_of(MAIN_UNIT, "add").unwrap();
    assert_eq!(
        before_contract.content_identity(),
        after_contract.content_identity(),
        "meaning did not change"
    );
    assert_eq!(before_call, after_call);
    assert_ne!(
        before_facts.provenance.name_span, after_facts.provenance.name_span,
        "provenance did change"
    );
}

/// **Trace 2 — body-only edit.**
///
/// `a + b` becomes `b + a`: same declared signature, same length, so spans do
/// not move either. Declared: parse and both index queries re-execute and
/// backdate; nothing semantic re-executes, and every published identity is
/// unchanged. This is the case an annotated contract must get right — a body
/// edit cannot change a declared contract.
#[test]
fn body_only_edit_changes_no_published_fact() {
    let mut session = tracer_session(true);
    settle(&session);
    let before_facts = session.callable_facts_of(MAIN_UNIT, "add").unwrap();
    let before_call = session.call_site_facts(MAIN_UNIT, 0).unwrap();
    session.take_trace();

    session.set_unit_text(
        MATH_UNIT,
        "pub fn add(a: int, b: int) -> int {\n    b + a\n}\n",
    );
    demand(&session);
    let trace = session.take_trace();

    assert_eq!(trace.executions("parsed_unit(app::math)"), 1);
    assert_eq!(trace.executions("declaration_index(app::math)"), 1);
    assert_eq!(trace.executions("unit_provenance(app::math)"), 1);
    assert_eq!(trace.executions("callable_contract"), 0, "{trace:#?}");
    assert_eq!(trace.executions("callable_facts"), 0);
    assert_eq!(trace.executions("call_site_facts"), 0);

    assert_eq!(
        before_facts.content_identity(),
        session
            .callable_facts_of(MAIN_UNIT, "add")
            .unwrap()
            .content_identity()
    );
    assert_eq!(before_call, session.call_site_facts(MAIN_UNIT, 0).unwrap());
}

/// **Trace 3 — signature edit.**
///
/// `a: int` becomes `a: string`. Declared: the index changes, so the contract,
/// the callable fact and the call-site check all re-execute, and the call site
/// gains a type-mismatch diagnostic it did not have. The definition's identity
/// does not change — it is the same declaration with a different contract.
#[test]
fn signature_edit_reruns_the_dependent_call_site_check() {
    let mut session = tracer_session(true);
    settle(&session);
    let before = session.callable_facts_of(MAIN_UNIT, "add").unwrap();
    assert!(
        session
            .call_site_facts(MAIN_UNIT, 0)
            .unwrap()
            .diagnostics
            .is_empty()
    );
    session.take_trace();

    session.set_unit_text(
        MATH_UNIT,
        "pub fn add(a: string, b: int) -> int {\n    b\n}\n",
    );
    demand(&session);
    let trace = session.take_trace();

    assert_eq!(trace.executions("declaration_index(app::math)"), 1);
    assert_eq!(trace.executions("callable_contract"), 1, "{trace:#?}");
    assert_eq!(trace.executions("callable_facts"), 1);
    assert_eq!(trace.executions("call_site_facts"), 1);

    let after = session.callable_facts_of(MAIN_UNIT, "add").unwrap();
    assert_eq!(after.identity(), before.identity(), "same declaration");
    assert_ne!(after.content_identity(), before.content_identity());
    assert_eq!(after.contract().params[0].ty, NormalizedType::String);

    let call = session.call_site_facts(MAIN_UNIT, 0).unwrap();
    let mismatch = call
        .diagnostics
        .iter()
        .find(|d| d.code == codes::CALL_ARGUMENT_TYPE_MISMATCH)
        .expect("the call site now disagrees with the published contract");
    assert_eq!(mismatch.arg("index"), Some("0"));
    assert_eq!(mismatch.arg("expected"), Some("string"));
    assert_eq!(mismatch.arg("actual"), Some("int"));
}

/// **Trace 4 — import retarget.**
///
/// `from app::math` becomes `from app::math2`, which declares a same-named
/// callable with the same contract but a different body. Declared: the
/// consuming unit re-parses and re-indexes, resolution re-runs and now names a
/// *different* definition identity; the newly reached unit is indexed for the
/// first time; the previously imported unit is not re-parsed or re-indexed.
#[test]
fn import_retarget_edit_changes_the_resolved_identity() {
    let mut session = tracer_session(true);
    settle(&session);
    let before = session.call_site_facts(MAIN_UNIT, 0).unwrap();
    assert_eq!(before.callee_identity(), Some(math_add_identity()));
    session.take_trace();

    session.set_unit_text(
        MAIN_UNIT,
        "from app::math2 use { add }\n\nlet total = add(1, 2)\n",
    );
    demand(&session);
    let trace = session.take_trace();

    assert_eq!(trace.executions("parsed_unit(app::main)"), 1);
    assert_eq!(trace.executions("declaration_index(app::main)"), 1);
    assert_eq!(trace.executions("resolve_callable"), 1, "{trace:#?}");
    assert_eq!(trace.executions("declaration_index(app::math2)"), 1);
    assert_eq!(
        trace.executions("declaration_index(app::math)"),
        0,
        "the abandoned unit is untouched"
    );
    assert_eq!(trace.executions("parsed_unit(app::math)"), 0);
    assert_eq!(trace.executions("call_site_facts"), 1);

    let after = session.call_site_facts(MAIN_UNIT, 0).unwrap();
    assert_eq!(
        after.callee_identity(),
        Some(DefinitionPath::top_level_callable(MATH2_UNIT, "add", 0).identity())
    );
    assert_ne!(after.callee_identity(), Some(math_add_identity()));
    // Identical contract, different definition: identity is not the contract.
    assert_eq!(
        session
            .callable_facts_of(MAIN_UNIT, "add")
            .unwrap()
            .contract(),
        &crate::CallableContract {
            type_params: vec![],
            params: vec![
                crate::ParamContract {
                    name: "a".into(),
                    ty: NormalizedType::Int,
                    by_reference: false,
                    mutable_reference: false,
                    is_const: false,
                    has_default: false,
                },
                crate::ParamContract {
                    name: "b".into(),
                    ty: NormalizedType::Int,
                    by_reference: false,
                    mutable_reference: false,
                    is_const: false,
                    has_default: false,
                },
            ],
            result: NormalizedType::Int,
            is_async: false,
            is_comptime: false,
        }
    );
}

/// **Trace 5 — alias edit.**
///
/// The import becomes `add as plus` and the call site is rewritten to `plus`.
/// Declared: the consuming unit re-indexes and resolution runs for the new
/// name, but because the alias resolves to the same definition, the callee's
/// contract and callable fact are *not* re-executed — the identity carried
/// straight through. Only the call site's own fact re-executes, because the
/// written spelling it records changed.
#[test]
fn alias_edit_preserves_identity_and_cuts_off_at_the_callee() {
    let mut session = tracer_session(true);
    settle(&session);
    let before = session.call_site_facts(MAIN_UNIT, 0).unwrap();
    session.take_trace();

    session.set_unit_text(
        MAIN_UNIT,
        "from app::math use { add as plus }\n\nlet total = plus(1, 2)\n",
    );
    session.callable_facts_of(MAIN_UNIT, "plus");
    session.call_site_facts(MAIN_UNIT, 0);
    let trace = session.take_trace();

    assert_eq!(trace.executions("declaration_index(app::main)"), 1);
    assert_eq!(trace.executions("resolve_callable"), 1);
    assert_eq!(
        trace.executions("declaration_index(app::math)"),
        0,
        "{trace:#?}"
    );
    assert_eq!(
        trace.executions("callable_contract"),
        0,
        "the aliased definition is the same definition"
    );
    assert_eq!(trace.executions("callable_facts"), 0);
    assert_eq!(trace.executions("call_site_facts"), 1);

    let after = session.call_site_facts(MAIN_UNIT, 0).unwrap();
    assert_eq!(after.callee_identity(), before.callee_identity());
    assert_eq!(after.written_name, "plus");
    assert_eq!(
        after.callee_contract_identity,
        before.callee_contract_identity
    );
}

/// **Trace 6 — local shadow.**
///
/// The consuming unit gains its own `fn add`, spelled the same as the import.
/// Declared: resolution re-runs and now names the local declaration — a
/// different identity — and reports the shadowed import; the imported unit is
/// not re-examined; the call site is re-checked against the local contract and
/// reports the arguments it now rejects.
#[test]
fn local_shadow_edit_resolves_to_a_different_definition() {
    let mut session = tracer_session(true);
    settle(&session);
    let before = session.call_site_facts(MAIN_UNIT, 0).unwrap();
    assert_eq!(before.callee_identity(), Some(math_add_identity()));
    session.take_trace();

    session.set_unit_text(
        MAIN_UNIT,
        "from app::math use { add }\n\nlet total = add(1, 2)\n\nfn add(a: string, b: string) -> string {\n    a\n}\n",
    );
    demand(&session);
    let trace = session.take_trace();

    assert_eq!(trace.executions("declaration_index(app::main)"), 1);
    assert_eq!(trace.executions("resolve_callable"), 1, "{trace:#?}");
    assert_eq!(
        trace.executions("declaration_index(app::math)"),
        0,
        "the shadowed unit is untouched"
    );
    assert_eq!(trace.executions("callable_contract"), 1);
    assert_eq!(trace.executions("call_site_facts"), 1);

    let after = session.call_site_facts(MAIN_UNIT, 0).unwrap();
    assert_eq!(
        after.callee_identity(),
        Some(DefinitionPath::top_level_callable(MAIN_UNIT, "add", 0).identity())
    );
    assert_ne!(after.callee_identity(), Some(math_add_identity()));
    assert_eq!(
        after
            .diagnostics
            .iter()
            .filter(|d| d.code == codes::CALL_ARGUMENT_TYPE_MISMATCH)
            .count(),
        2
    );
    assert!(
        after
            .diagnostics
            .iter()
            .any(|d| d.code == codes::IMPORT_SHADOWED_BY_LOCAL_DECLARATION)
    );
}

// ---------------------------------------------------------------------------
// Query-memory budget (R16)
// ---------------------------------------------------------------------------

/// The initial query-memory budget is measured, not assumed.
///
/// What is counted is Salsa's own bookkeeping for this session — struct fields,
/// memo slots and metadata — as reported by `Database::memory_usage()`. Heap
/// reachable *through* a memoized value (the `Arc<Program>` a parse holds, for
/// instance) is not: Salsa can only report it for ingredients that declare a
/// `heap_size` function, and this slice declares none. The budget is therefore
/// a tripwire on the seam's bookkeeping growth, not a bound on total process
/// memory, and `docs/program/adr011-012/salsa-seam.md` says so too.
///
/// Measured for the three-unit tracer program at the recording revision: 704
/// struct bytes + 1,848 memo bytes = 2,552 bytes. The 16 KiB ceiling leaves
/// room for ordinary growth and fails on a structural regression.
#[test]
fn query_memory_stays_within_the_recorded_budget() {
    let session = tracer_session(false);
    demand(&session);
    let report = session.query_memory();
    assert!(
        report.total_bytes() < 16 * 1024,
        "tracer session Salsa bookkeeping is {} bytes, over the recorded 16 KiB budget: {:#?}",
        report.total_bytes(),
        report
    );
    assert!(
        report
            .queries
            .iter()
            .any(|entry| entry.query.contains("callable_facts")),
        "the report names its queries: {:#?}",
        report
    );
}
