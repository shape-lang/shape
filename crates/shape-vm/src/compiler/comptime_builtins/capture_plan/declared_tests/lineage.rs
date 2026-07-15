use super::*;

#[test]
fn nested_move_preserves_original_lineage_and_semantic_type_by_ordinal() {
    let compiler = compile(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read() -> int { let total = 40
      let outer = |; move total| { let inner = |; move total| total + 2
        inner() }
      outer() } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
job.read()
"#,
    );
    let mut packs = compiler
        .closure_capture_packs
        .iter()
        .filter(|pack| pack.descriptors.len() == 1 && pack.descriptors[0].name == "total");
    let outer = packs.next().expect("outer total capture");
    let inner = packs.next().expect("inner total capture");
    assert!(packs.next().is_none(), "exactly two total capture packs");

    assert_eq!(outer.descriptors[0].access, CaptureAccess::Param);
    assert_eq!(inner.descriptors[0].access, CaptureAccess::Param);
    assert_eq!(
        outer.descriptors[0].ownership,
        Some(BindingOwnershipClass::OwnedImmutable)
    );
    assert_eq!(
        inner.descriptors[0].ownership,
        Some(BindingOwnershipClass::OwnedImmutable)
    );
    assert_eq!(outer.descriptors[0].declared, Some(CaptureMode::Move));
    assert_eq!(inner.descriptors[0].declared, Some(CaptureMode::Move));
    assert_ne!(
        outer.descriptors[0].target, inner.descriptors[0].target,
        "the nested descriptor should expose its immediate synthetic slot"
    );
    assert!(outer.descriptors[0].binding_lineage.is_some());
    assert_eq!(
        outer.descriptors[0].binding_lineage, inner.descriptors[0].binding_lineage,
        "the nested immediate slot must not replace original binding identity"
    );
    assert!(matches!(
        &outer.descriptors[0].semantic_type,
        CaptureSemanticEvidence::Exact(_)
    ));
    assert_eq!(
        outer.descriptors[0].semantic_type, inner.descriptors[0].semantic_type,
        "immutable/move forwarding must preserve frozen semantic type"
    );
}
