//! Complete authored-source graph for one compiler-issued capture binding.

use shape_vm::compiler::{GeneratedCaptureQuery, GeneratedCaptureSite, SourceAnchor};

/// Collect every verified authored anchor joined by the identities at `site`.
///
/// `None` is a quarantine result: at least one occurrence of the selected
/// binding has no exact source map, so returning a partial graph would make
/// references and rename disagree about the binding.
pub(super) fn complete_binding_anchors(
    query: &GeneratedCaptureQuery,
    site: &GeneratedCaptureSite<'_>,
) -> Option<Vec<SourceAnchor>> {
    let mut identities: Vec<_> = site
        .captures()
        .iter()
        .map(|capture| capture.identity().clone())
        .collect();
    identities.sort_by_key(|identity| identity.canonical_descriptor());
    identities.dedup();

    let mut anchors = Vec::new();
    for identity in identities {
        for occurrence in query.captures_for_binding(&identity) {
            let source = occurrence.source_map()?;
            anchors.push(source.binding());
            anchors.push(source.declaration());
            anchors.extend(source.uses());
        }
    }
    anchors.sort_by_key(|anchor| (anchor.file_id(), anchor.span().start, anchor.span().end));
    anchors.dedup();
    Some(anchors)
}
