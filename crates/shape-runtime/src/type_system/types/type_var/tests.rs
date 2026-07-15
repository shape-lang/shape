use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use shape_ast::ast::TypeAnnotation;

use super::*;
use crate::type_system::semantic::TypeVarId;

fn hash(variable: &TypeVar) -> u64 {
    let mut hasher = DefaultHasher::new();
    variable.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn raw_string_matching_declared_carrier_is_legacy_not_authority() {
    let mut generator = TypeVarGen::new();
    let declared = TypeVar::declared(generator.fresh_declared_owner(), 0, "T");
    let TypeAnnotation::Basic(carrier) = tyvar_to_annotation(&declared) else {
        panic!("type-variable carrier must be a basic annotation")
    };

    let raw_carrier = TypeVar::new(carrier);
    let raw_declared_encoding = TypeVar::new("\u{1}decl:1:2:0:T".to_string());

    assert_ne!(raw_carrier, declared);
    assert!(raw_carrier.declared_provenance().is_none());
    assert_eq!(raw_carrier.presentation_name(), REDACTED_TYPE_VAR_NAME);
    assert!(!format!("{raw_carrier:?}").contains(TYVAR_ANNOTATION_PREFIX));
    assert_ne!(raw_declared_encoding, declared);
    assert!(raw_declared_encoding.declared_provenance().is_none());
    assert_eq!(
        raw_declared_encoding.presentation_name(),
        REDACTED_TYPE_VAR_NAME
    );
}

#[test]
fn fabricated_annotation_cannot_recover_declared_authority() {
    let fabricated = TypeAnnotation::Basic(format!(
        "{TYVAR_ANNOTATION_PREFIX}{CARRIER_VERSION}:d:0000000000000001:00000000:00000000:54:{}",
        "00".repeat(32)
    ));

    assert!(annotation_as_tyvar(&fabricated).is_none());
    let nested = TypeAnnotation::Tuple(vec![
        TypeAnnotation::Basic("int".to_string()),
        TypeAnnotation::Array(Box::new(fabricated.clone())),
    ]);
    assert!(annotation_contains_reserved_type_var_carrier(&fabricated));
    assert!(annotation_contains_reserved_type_var_carrier(&nested));
    assert!(!annotation_contains_reserved_type_var_carrier(
        &TypeAnnotation::Basic("ordinary".to_string())
    ));
}

#[test]
fn tampered_variant_payload_and_mac_all_fail_closed() {
    let mut generator = TypeVarGen::new();
    let declared = TypeVar::declared(generator.fresh_declared_owner(), 0, "T");
    let TypeAnnotation::Basic(carrier) = tyvar_to_annotation(&declared) else {
        panic!("type-variable carrier must be a basic annotation")
    };
    let encoded = carrier
        .strip_prefix(TYVAR_ANNOTATION_PREFIX)
        .expect("carrier prefix");
    let (payload, authentication) = encoded.rsplit_once(':').expect("carrier mac");

    let changed_variant = payload.replacen(":d:", ":l:", 1);
    let changed_payload = format!("{payload}00");
    let replacement = if authentication.starts_with('0') {
        '1'
    } else {
        '0'
    };
    let mut changed_authentication = authentication.to_string();
    changed_authentication.replace_range(..1, &replacement.to_string());

    for forged in [
        format!("{TYVAR_ANNOTATION_PREFIX}{changed_variant}:{authentication}"),
        format!("{TYVAR_ANNOTATION_PREFIX}{changed_payload}:{authentication}"),
        format!("{TYVAR_ANNOTATION_PREFIX}{payload}:{changed_authentication}"),
    ] {
        assert!(
            annotation_as_tyvar(&TypeAnnotation::Basic(forged)).is_none(),
            "modified carrier must not recover authority"
        );
    }
}

#[test]
fn compiler_issued_declared_round_trip_retains_exact_identity() {
    let mut generator = TypeVarGen::new();
    let owner = generator.fresh_declared_owner();
    let declared = TypeVar::declared(owner, 7, "Élément:值");

    let recovered = annotation_as_tyvar(&tyvar_to_annotation(&declared))
        .expect("compiler-issued carrier must authenticate");
    let provenance = recovered
        .declared_provenance()
        .expect("declared provenance must survive the carrier");

    assert_eq!(recovered, declared);
    assert_eq!(hash(&recovered), hash(&declared));
    assert_eq!(provenance.owner(), owner);
    assert_eq!(provenance.ordinal(), 7);
    assert_eq!(provenance.source_name(), "Élément:值");
}

#[test]
fn legacy_and_inference_hole_behaviour_remains_distinct() {
    let legacy = TypeVar::new("T42".to_string());
    let legacy_round_trip = annotation_as_tyvar(&tyvar_to_annotation(&legacy))
        .expect("legacy carrier must authenticate");
    assert_eq!(legacy_round_trip, legacy);
    assert_eq!(legacy.presentation_name(), "T42");
    assert_eq!(legacy.legacy_semantic_id(), Some(TypeVarId(42)));
    assert!(legacy.declared_provenance().is_none());

    let mut generator = TypeVarGen::new();
    let hole = generator.fresh_var();
    let hole_round_trip = annotation_as_tyvar(&tyvar_to_annotation(&hole))
        .expect("inference-hole carrier must authenticate");
    assert_eq!(hole_round_trip, hole);
    assert_eq!(hole.presentation_name(), "T0");
    assert!(hole.legacy_semantic_id().is_none());
    assert!(hole.declared_provenance().is_none());
    assert_ne!(hole, TypeVar::new("T0".to_string()));
}

#[test]
fn declared_source_renaming_does_not_change_owner_ordinal_identity() {
    let mut generator = TypeVarGen::new();
    let owner = generator.fresh_declared_owner();
    let original = TypeVar::declared(owner, 3, "T");
    let renamed = TypeVar::declared(owner, 3, "Renamed");

    assert_eq!(original, renamed);
    assert_eq!(hash(&original), hash(&renamed));
    assert_eq!(original.presentation_name(), "T");
    assert_eq!(renamed.presentation_name(), "Renamed");
}

#[test]
fn debug_output_exposes_presentation_only() {
    let mut generator = TypeVarGen::new();
    assert_eq!(format!("{generator:?}"), "TypeVarGen { .. }");
    let declared = TypeVar::declared(generator.fresh_declared_owner(), 0, "Element");
    let rendered = format!("{declared:?}");

    assert_eq!(rendered, "TypeVar(\"Element\")");
    assert!(!rendered.contains(TYVAR_ANNOTATION_PREFIX));
    assert!(!rendered.contains(CARRIER_VERSION));
}
