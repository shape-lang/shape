use std::collections::{HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use super::*;

fn path(leaf: u32) -> GeneratedNodePath {
    GeneratedNodePath::decl_root("extend:Job")
        .child("method:read")
        .child(format!("closure:{leaf}"))
}

fn issue(
    issuer: &GeneratedNodeIssuer,
    fingerprint: (i64, i64),
    path: GeneratedNodePath,
    file_id: u16,
    span: Span,
    owner: &str,
) -> GeneratedNodeOrigin {
    issuer.issue(
        GeneratedExpansionFingerprint::from_components(fingerprint.0, fingerprint.1),
        path,
        file_id,
        span,
        owner.to_string(),
    )
}

fn semantic_hash(origin: &GeneratedNodeOrigin) -> u64 {
    let mut hasher = DefaultHasher::new();
    origin.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn authority_is_compiler_instance_scoped() {
    let compiler = GeneratedNodeIssuer::new();
    let foreign = GeneratedNodeIssuer::new();
    let origin = issue(
        &foreign,
        (1, 2),
        path(0),
        3,
        Span { start: 5, end: 9 },
        "Job.read",
    );
    let same_identity = issue(
        &compiler,
        (1, 2),
        path(0),
        3,
        Span { start: 5, end: 9 },
        "Job.read",
    );
    assert!(foreign.recognizes(&origin));
    assert!(!compiler.recognizes(&origin));
    assert_eq!(
        origin, same_identity,
        "trust capability is not semantic identity"
    );
}

#[test]
fn source_anchor_and_owner_display_do_not_change_identity_or_hash() {
    let compiler = GeneratedNodeIssuer::new();
    let first = issue(
        &compiler,
        (1, 2),
        path(0),
        3,
        Span { start: 5, end: 9 },
        "Job.read",
    );
    let relocated = issue(
        &compiler,
        (1, 2),
        path(0),
        99,
        Span {
            start: 100,
            end: 140,
        },
        "renamed diagnostic owner",
    );

    assert_eq!(first, relocated);
    assert_eq!(semantic_hash(&first), semantic_hash(&relocated));
    assert_ne!(first.anchor(), relocated.anchor());
    assert_ne!(first.owner_display(), relocated.owner_display());
}

#[test]
fn fingerprint_and_structural_path_are_both_identity() {
    let compiler = GeneratedNodeIssuer::new();
    let baseline = issue(
        &compiler,
        (1, 2),
        path(0),
        3,
        Span { start: 5, end: 9 },
        "Job.read",
    );
    let other_expansion = issue(
        &compiler,
        (1, 3),
        path(0),
        3,
        Span { start: 5, end: 9 },
        "Job.read",
    );
    let sibling = issue(
        &compiler,
        (1, 2),
        path(1),
        3,
        Span { start: 5, end: 9 },
        "Job.read",
    );

    assert_ne!(baseline, other_expansion);
    assert_ne!(baseline, sibling);
    assert_eq!(
        HashSet::from([baseline, other_expansion, sibling]).len(),
        3,
        "expansion and path differences must remain distinct hash keys"
    );
}

#[test]
fn malformed_rendered_segments_cannot_form_an_issuable_path() {
    assert!(GeneratedNodePath::try_from_rendered_segments([""]).is_err());
    assert!(GeneratedNodePath::try_from_rendered_segments(["closure:0/closure:1"]).is_err());
    assert!(GeneratedNodePath::try_from_rendered_segments(["closure:\n0"]).is_err());
}

#[test]
fn path_encoding_preserves_empty_and_root_only_shapes() {
    assert!(GeneratedNodePath::empty().segments().is_empty());
    let root = GeneratedNodePath::decl_root("fn:generated");
    assert_eq!(root.segments(), ["fn:generated"]);
    assert_eq!(root.parent(), Some(GeneratedNodePath::empty()));
}

#[test]
fn serde_round_trip_preserves_presentation_but_erases_authority() {
    let compiler = GeneratedNodeIssuer::new();
    let issued = issue(
        &compiler,
        (1, 2),
        path(0),
        3,
        Span { start: 5, end: 9 },
        "Job.read",
    );
    assert!(compiler.recognizes(&issued));

    let json = serde_json::to_string(&issued).unwrap();
    assert!(json.contains("\"expansion_high\":1"));
    assert!(json.contains("\"node_path\":[\"extend:Job\",\"method:read\",\"closure:0\"]"));
    let decoded: GeneratedNodeOrigin = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded.identity(), issued.identity());
    assert_eq!(decoded.anchor(), issued.anchor());
    assert_eq!(decoded.owner_display(), issued.owner_display());
    assert!(!compiler.recognizes(&decoded));
}
