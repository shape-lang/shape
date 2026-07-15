use super::*;
use shape_ast::ast::{CaptureMode, GeneratedNodeOrigin, Span};
use shape_runtime::comptime_reflection::FrozenTypeCategory;
use shape_runtime::type_system::GeneratedNodeKey;
use shape_value::v2::closure_layout::CaptureKind;

use super::super::super::CaptureBindingLineage;
use super::super::specialization::normalize_semantic_presentations;
use super::super::{
    GeneratedCaptureBindingIdentity, GeneratedCapturePosition, GeneratedCaptureQuery,
    GeneratedCaptureSemanticType, GeneratedCaptureSpecialization,
    GeneratedCaptureSpecializationIdentity,
};

#[test]
fn valid_structural_specializations_merge_without_compilation_order_dependence() {
    let source_map = source_map(10, 20, 30);
    let occurrence = occurrence(4, 8, 0);
    let first = view(
        occurrence.clone(),
        source_map.clone(),
        "total",
        specialization((1, 1), (11, 11), "z-int", "fn() -> int"),
    );
    let second = view(
        occurrence,
        source_map,
        "total",
        specialization((1, 1), (12, 12), "a-int", "fn() -> string"),
    );

    let forward = aggregate([first.clone(), second.clone()]);
    let reverse = aggregate([second, first]);
    assert!(forward.issues().is_empty());
    assert_eq!(forward.captures().len(), 1);
    assert_eq!(forward.captures()[0].specializations().len(), 2);
    assert!(
        forward.captures()[0]
            .specializations()
            .iter()
            .all(|specialization| specialization.capture_type().presentation() == "a-int")
    );
    assert!(
        forward.captures()[0]
            .specializations()
            .iter()
            .all(|specialization| {
                specialization.identity().capture_types()[0].presentation() == "a-int"
            })
    );
    assert_eq!(
        specialization_descriptors(&forward),
        specialization_descriptors(&reverse),
    );
}

#[test]
fn equal_specialization_identity_merges_diagnostic_text_deterministically() {
    let source_map = source_map(10, 20, 30);
    let occurrence = occurrence(4, 8, 0);
    let later = view(
        occurrence.clone(),
        source_map.clone(),
        "total",
        specialization((1, 1), (11, 11), "z-int", "z-fn"),
    );
    let earlier = view(
        occurrence,
        source_map,
        "total",
        specialization((1, 1), (11, 11), "a-int", "a-fn"),
    );

    for query in [
        aggregate([later.clone(), earlier.clone()]),
        aggregate([earlier, later]),
    ] {
        assert!(
            query.issues().is_empty(),
            "diagnostic presentation is not an artifact contract",
        );
        let specializations = query.captures()[0].specializations();
        assert_eq!(specializations.len(), 1);
        assert_eq!(specializations[0].capture_type().presentation(), "a-int");
        assert_eq!(
            specializations[0].identity().callable_type().presentation(),
            "a-fn",
        );
    }
}

#[test]
fn conflicting_contract_for_one_occurrence_has_no_retained_winner() {
    let source_map = source_map(10, 20, 30);
    let occurrence = occurrence(4, 8, 0);
    let first = view(
        occurrence.clone(),
        source_map.clone(),
        "total",
        specialization((1, 1), (11, 11), "int", "fn() -> int"),
    );
    let second = view(
        occurrence,
        source_map,
        "different-name",
        specialization((1, 1), (11, 11), "int", "fn() -> int"),
    );
    let query = aggregate([first, second]);

    assert!(query.captures().is_empty());
    assert_eq!(conflict_count(&query), 1);
    assert!(matches!(
        query.capture_at(0, 22),
        Some(GeneratedCapturePosition::Unavailable),
    ));
}

#[test]
fn incompatible_occurrences_at_one_authored_site_quarantine_every_source_map() {
    let first_map = source_map(10, 20, 30);
    let second_map = source_map(40, 20, 50);
    let first = view(
        occurrence(4, 8, 0),
        first_map,
        "total",
        specialization((1, 1), (11, 11), "int", "fn() -> int"),
    );
    let second = view(
        occurrence(5, 9, 0),
        second_map,
        "total",
        specialization((1, 1), (11, 11), "int", "fn() -> int"),
    );
    let query = aggregate([first, second]);

    assert!(query.captures().is_empty());
    assert_eq!(conflict_count(&query), 1);
    for offset in [12, 22, 42, 52] {
        assert!(matches!(
            query.capture_at(0, offset),
            Some(GeneratedCapturePosition::Unavailable),
        ));
    }
}

#[test]
fn poisoned_occurrence_is_omitted_and_quarantines_every_real_source_site() {
    let occurrence = occurrence(4, 8, 0);
    let source_map = source_map(10, 20, 30);
    let mut accumulator = CaptureAccumulator::new();
    accumulator.poison(
        occurrence.clone(),
        Some(source_map),
        Some(anchor(40, 50)),
        "missing semantic evidence",
    );
    // Later observations cannot resurrect a poisoned occurrence or add a
    // second, order-dependent diagnostic.
    accumulator.poison(occurrence, None, Some(anchor(60, 70)), "later observation");
    let query = from_aggregated(accumulator.finish());

    assert!(query.captures().is_empty());
    assert_eq!(conflict_count(&query), 1);
    for offset in [12, 22, 32] {
        assert!(matches!(
            query.capture_at(0, offset),
            Some(GeneratedCapturePosition::Unavailable),
        ));
    }
}

#[test]
fn poisoned_reasons_and_anchor_are_independent_of_observation_order() {
    let source_map = source_map(10, 20, 30);
    let forward = aggregate_poisoned([
        (Some(source_map.clone()), 60, "zeta evidence failure"),
        (None, 40, "alpha evidence failure"),
    ]);
    let reverse = aggregate_poisoned([
        (None, 40, "alpha evidence failure"),
        (Some(source_map), 60, "zeta evidence failure"),
    ]);

    assert_eq!(forward.issues(), reverse.issues());
    assert_eq!(
        forward.issues()[0].message(),
        format!(
            "[C0911] alpha evidence failure; zeta evidence failure: {}",
            occurrence(4, 8, 0).canonical_descriptor(),
        ),
    );
    assert_eq!(forward.issues()[0].anchor(), Some(anchor(10, 15)));
}

fn aggregate_poisoned(
    observations: impl IntoIterator<Item = (Option<GeneratedCaptureSourceMap>, usize, &'static str)>,
) -> GeneratedCaptureQuery {
    let occurrence = occurrence(4, 8, 0);
    let mut accumulator = CaptureAccumulator::new();
    for (source_map, anchor_start, reason) in observations {
        accumulator.poison(
            occurrence.clone(),
            source_map,
            Some(anchor(anchor_start, anchor_start + 5)),
            reason,
        );
    }
    from_aggregated(accumulator.finish())
}

fn aggregate(
    views: impl IntoIterator<Item = GeneratedCaptureDescriptorView>,
) -> GeneratedCaptureQuery {
    let mut accumulator = CaptureAccumulator::new();
    for view in views {
        accumulator.insert(view);
    }
    from_aggregated(accumulator.finish())
}

fn from_aggregated(mut aggregated: AggregatedCaptures) -> GeneratedCaptureQuery {
    normalize_semantic_presentations(&mut aggregated.captures);
    GeneratedCaptureQuery {
        captures: aggregated.captures,
        issues: aggregated.issues,
        quarantined_source_maps: aggregated.quarantined_source_maps,
    }
}

fn view(
    occurrence_identity: GeneratedCaptureOccurrenceIdentity,
    source_map: GeneratedCaptureSourceMap,
    display_name: &str,
    specialization: GeneratedCaptureSpecialization,
) -> GeneratedCaptureDescriptorView {
    GeneratedCaptureDescriptorView {
        identity: GeneratedCaptureBindingIdentity::from_binding_lineage(
            &CaptureBindingLineage::ModuleBinding {
                file_id: 0,
                slot: 3,
            },
        ),
        occurrence_identity,
        display_name: display_name.to_string(),
        mode: CaptureMode::Share,
        specializations: vec![specialization],
        owner_display: "Job.read".to_string(),
        owner_node_path: "method:read/closure:0".to_string(),
        application: Some(anchor(70, 80)),
        source_map: Some(source_map),
    }
}

fn specialization(
    capture_identity: (i64, i64),
    callable_identity: (i64, i64),
    capture_presentation: &str,
    callable_presentation: &str,
) -> GeneratedCaptureSpecialization {
    let capture_type = GeneratedCaptureSemanticType::for_test(
        FrozenTypeCategory::Primitive,
        capture_identity,
        capture_presentation,
    );
    let callable_type = GeneratedCaptureSemanticType::for_test(
        FrozenTypeCategory::Callable,
        callable_identity,
        callable_presentation,
    );
    let identity = GeneratedCaptureSpecializationIdentity::for_test(
        vec![capture_type.clone()],
        vec![Some(CaptureMode::Share)],
        vec![CaptureKind::Shared],
        callable_type,
    );
    GeneratedCaptureSpecialization::for_test(identity, capture_type)
}

fn occurrence(
    high: i64,
    low: i64,
    descriptor_ordinal: usize,
) -> GeneratedCaptureOccurrenceIdentity {
    let origin: GeneratedNodeOrigin = serde_json::from_value(serde_json::json!({
        "expansion_high": high,
        "expansion_low": low,
        "node_path": ["method:read", "closure:0"],
        "anchor_file_id": 0,
        "anchor_span": { "start": 70, "end": 80 },
        "owner_display": "Job.read",
    }))
    .expect("serialized provenance data decodes without compiler authority");
    GeneratedCaptureOccurrenceIdentity {
        node: GeneratedNodeKey::from_origin(&origin),
        descriptor_ordinal,
    }
}

fn source_map(binding: usize, declaration: usize, use_site: usize) -> GeneratedCaptureSourceMap {
    GeneratedCaptureSourceMap {
        binding: anchor(binding, binding + 5),
        declaration: anchor(declaration, declaration + 5),
        uses: vec![anchor(use_site, use_site + 5)],
    }
}

fn specialization_descriptors(query: &GeneratedCaptureQuery) -> Vec<String> {
    query.captures()[0]
        .specializations()
        .iter()
        .map(|specialization| specialization.identity().canonical_descriptor())
        .collect()
}

fn conflict_count(query: &GeneratedCaptureQuery) -> usize {
    query
        .issues()
        .iter()
        .filter(|issue| issue.code() == GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE)
        .count()
}

fn anchor(start: usize, end: usize) -> SourceAnchor {
    SourceAnchor::new(0, Span::new(start, end)).expect("test anchor is valid")
}
