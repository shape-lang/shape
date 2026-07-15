//! Conflict-aware aggregation for generated capture query artifacts.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::{
    GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE, GeneratedCaptureDescriptorView,
    GeneratedCaptureOccurrenceIdentity, GeneratedCaptureQueryIssue, GeneratedCaptureSourceMap,
};
use crate::compiler::SourceAnchor;

enum CaptureState {
    Active(GeneratedCaptureDescriptorView),
    Poisoned {
        source_maps: Vec<GeneratedCaptureSourceMap>,
        anchors: Vec<SourceAnchor>,
        reasons: BTreeSet<String>,
    },
}

/// Aggregates monomorphized compiler artifacts without ever retaining an
/// arbitrary winner after a structural disagreement.
pub(super) struct CaptureAccumulator {
    states: HashMap<GeneratedCaptureOccurrenceIdentity, CaptureState>,
    issues: Vec<GeneratedCaptureQueryIssue>,
}

pub(super) struct AggregatedCaptures {
    pub(super) captures: Vec<GeneratedCaptureDescriptorView>,
    pub(super) issues: Vec<GeneratedCaptureQueryIssue>,
    pub(super) quarantined_source_maps: Vec<GeneratedCaptureSourceMap>,
}

impl CaptureAccumulator {
    pub(super) fn new() -> Self {
        Self {
            states: HashMap::new(),
            issues: Vec::new(),
        }
    }

    pub(super) fn insert(&mut self, view: GeneratedCaptureDescriptorView) {
        let occurrence = view.occurrence_identity.clone();
        match self.states.remove(&occurrence) {
            None => {
                self.states.insert(occurrence, CaptureState::Active(view));
            }
            Some(CaptureState::Active(mut existing)) => {
                let same_contract = existing.same_occurrence_contract(&view);
                let specialization = match view.specializations.as_slice() {
                    [specialization] => Some(specialization.clone()),
                    _ => None,
                };
                if same_contract
                    && specialization.is_some_and(|specialization| {
                        existing.merge_specialization(specialization).is_ok()
                    })
                {
                    self.states
                        .insert(occurrence, CaptureState::Active(existing));
                } else {
                    let source_maps = source_maps([existing.source_map, view.source_map]);
                    self.poison_new(
                        occurrence,
                        source_maps,
                        [existing.application, view.application]
                            .into_iter()
                            .flatten()
                            .collect(),
                        "generated capture artifacts disagree for one structural occurrence"
                            .to_string(),
                    );
                }
            }
            Some(CaptureState::Poisoned {
                mut source_maps,
                mut anchors,
                reasons,
            }) => {
                extend_source_maps(&mut source_maps, view.source_map);
                anchors.extend(view.application);
                self.states.insert(
                    occurrence,
                    CaptureState::Poisoned {
                        source_maps,
                        anchors,
                        reasons,
                    },
                );
            }
        }
    }

    /// Quarantine an occurrence whose compiler evidence is incomplete before
    /// a publishable view can be constructed.
    pub(super) fn poison(
        &mut self,
        occurrence: GeneratedCaptureOccurrenceIdentity,
        source_map: Option<GeneratedCaptureSourceMap>,
        anchor: Option<SourceAnchor>,
        reason: impl Into<String>,
    ) {
        let reason = reason.into();
        match self.states.remove(&occurrence) {
            None => {
                let source_maps = source_maps([source_map]);
                self.poison_new(
                    occurrence,
                    source_maps,
                    anchor.into_iter().collect(),
                    reason,
                );
            }
            Some(CaptureState::Active(existing)) => {
                let source_maps = source_maps([existing.source_map, source_map]);
                let anchors = [anchor, existing.application]
                    .into_iter()
                    .flatten()
                    .collect();
                self.poison_new(occurrence, source_maps, anchors, reason);
            }
            Some(CaptureState::Poisoned {
                mut source_maps,
                mut anchors,
                mut reasons,
            }) => {
                extend_source_maps(&mut source_maps, source_map);
                anchors.extend(anchor);
                reasons.insert(reason);
                self.states.insert(
                    occurrence,
                    CaptureState::Poisoned {
                        source_maps,
                        anchors,
                        reasons,
                    },
                );
            }
        }
    }

    pub(super) fn finish(mut self) -> AggregatedCaptures {
        self.quarantine_conflicting_source_contracts();

        let mut captures = Vec::new();
        let mut quarantined_source_maps = Vec::new();
        for (occurrence, state) in self.states {
            match state {
                CaptureState::Active(view) => captures.push(view),
                CaptureState::Poisoned {
                    source_maps,
                    anchors,
                    reasons,
                } => {
                    if !reasons.is_empty() {
                        let anchor = best_anchor(
                            anchors
                                .into_iter()
                                .map(Some)
                                .chain(source_map_anchors(&source_maps)),
                        );
                        self.issues.push(GeneratedCaptureQueryIssue {
                            code: GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE,
                            message: format!(
                                "[C0911] {}: {}",
                                reasons.into_iter().collect::<Vec<_>>().join("; "),
                                occurrence.canonical_descriptor(),
                            ),
                            anchor,
                        });
                    }
                    quarantined_source_maps.extend(source_maps);
                }
            }
        }
        captures.sort_by(|left, right| {
            left.occurrence_identity
                .canonical_descriptor()
                .cmp(&right.occurrence_identity.canonical_descriptor())
        });
        sort_source_maps(&mut quarantined_source_maps);
        self.issues.sort_by(|left, right| {
            (
                left.code,
                left.message.as_str(),
                left.anchor.map(anchor_key),
            )
                .cmp(&(
                    right.code,
                    right.message.as_str(),
                    right.anchor.map(anchor_key),
                ))
        });
        self.issues.dedup();
        AggregatedCaptures {
            captures,
            issues: self.issues,
            quarantined_source_maps,
        }
    }

    fn poison_new(
        &mut self,
        occurrence: GeneratedCaptureOccurrenceIdentity,
        source_maps: Vec<GeneratedCaptureSourceMap>,
        anchors: Vec<SourceAnchor>,
        reason: String,
    ) {
        self.states.insert(
            occurrence,
            CaptureState::Poisoned {
                source_maps,
                anchors,
                reasons: BTreeSet::from([reason]),
            },
        );
    }

    fn quarantine_conflicting_source_contracts(&mut self) {
        let mut contracts: HashMap<
            SourceAnchor,
            (
                String,
                shape_ast::ast::CaptureMode,
                GeneratedCaptureSourceMap,
            ),
        > = HashMap::new();
        let mut conflicts = HashSet::new();
        for state in self.states.values() {
            let CaptureState::Active(view) = state else {
                continue;
            };
            let Some(source_map) = view.source_map.clone() else {
                continue;
            };
            let site = source_map.declaration();
            let contract = (view.display_name.clone(), view.mode, source_map);
            if contracts
                .insert(site, contract.clone())
                .is_some_and(|existing| existing != contract)
            {
                conflicts.insert(site);
            }
        }

        for site in conflicts {
            let affected: Vec<_> = self
                .states
                .iter()
                .filter_map(|(occurrence, state)| match state {
                    CaptureState::Active(view)
                        if view
                            .source_map
                            .as_ref()
                            .is_some_and(|source_map| source_map.declaration() == site) =>
                    {
                        Some(occurrence.clone())
                    }
                    _ => None,
                })
                .collect();
            for occurrence in affected {
                let Some(CaptureState::Active(view)) = self.states.remove(&occurrence) else {
                    continue;
                };
                self.states.insert(
                    occurrence,
                    CaptureState::Poisoned {
                        source_maps: source_maps([view.source_map]),
                        anchors: vec![site],
                        reasons: BTreeSet::new(),
                    },
                );
            }
            self.issues.push(GeneratedCaptureQueryIssue {
                code: GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE,
                message: "[C0911] generated capture artifacts map incompatible contracts to one authored source site"
                    .to_string(),
                anchor: Some(site),
            });
        }
    }
}

fn source_maps(
    maps: impl IntoIterator<Item = Option<GeneratedCaptureSourceMap>>,
) -> Vec<GeneratedCaptureSourceMap> {
    let mut maps: Vec<_> = maps.into_iter().flatten().collect();
    sort_source_maps(&mut maps);
    maps
}

fn extend_source_maps(
    maps: &mut Vec<GeneratedCaptureSourceMap>,
    source_map: Option<GeneratedCaptureSourceMap>,
) {
    maps.extend(source_map);
    sort_source_maps(maps);
}

fn sort_source_maps(maps: &mut Vec<GeneratedCaptureSourceMap>) {
    maps.sort_by_key(source_map_key);
    maps.dedup();
}

fn source_map_key(
    source_map: &GeneratedCaptureSourceMap,
) -> (
    (u16, usize, usize),
    (u16, usize, usize),
    Vec<(u16, usize, usize)>,
) {
    let declaration = source_map.declaration();
    (
        anchor_key(declaration),
        anchor_key(source_map.binding()),
        source_map.uses().iter().copied().map(anchor_key).collect(),
    )
}

fn source_map_anchors(
    maps: &[GeneratedCaptureSourceMap],
) -> impl Iterator<Item = Option<SourceAnchor>> + '_ {
    maps.iter()
        .flat_map(|source_map| [Some(source_map.declaration()), Some(source_map.binding())])
}

fn best_anchor(anchors: impl IntoIterator<Item = Option<SourceAnchor>>) -> Option<SourceAnchor> {
    anchors
        .into_iter()
        .flatten()
        .min_by_key(|anchor| anchor_key(*anchor))
}

fn anchor_key(anchor: SourceAnchor) -> (u16, usize, usize) {
    (anchor.file_id(), anchor.span().start, anchor.span().end)
}

#[cfg(test)]
#[path = "aggregation/tests.rs"]
mod tests;
