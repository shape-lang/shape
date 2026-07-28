use std::collections::{BTreeMap, BTreeSet};

use shape_value::v2::ConcreteType;

use crate::type_tracking::BindingStorageClass;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum BindingKey {
    Local(u16),
    ModuleBinding(u16),
}

impl BindingKey {
    pub(super) fn description(self) -> String {
        match self {
            Self::Local(slot) => format!("Local({slot})"),
            Self::ModuleBinding(slot) => format!("ModuleBinding({slot})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReferenceClass {
    Value,
    SharedReference { referent: Option<ConcreteType> },
    ExclusiveReference { referent: Option<ConcreteType> },
}

impl ReferenceClass {
    pub(super) fn description(&self) -> String {
        match self {
            Self::Value => "Value".to_string(),
            Self::SharedReference { referent: None } => "SharedReference<?>".to_string(),
            Self::SharedReference {
                referent: Some(referent),
            } => format!("SharedReference<{referent:?}>"),
            Self::ExclusiveReference { referent: None } => "ExclusiveReference<?>".to_string(),
            Self::ExclusiveReference {
                referent: Some(referent),
            } => format!("ExclusiveReference<{referent:?}>"),
        }
    }

    pub(super) fn is_reference(&self) -> bool {
        !matches!(self, Self::Value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReferenceFlowEvidence {
    pub(super) class: ReferenceClass,
    pub(super) storage: Option<BindingStorageClass>,
}

impl ReferenceFlowEvidence {
    pub(super) fn description(&self) -> String {
        let storage = self
            .storage
            .map(|storage| format!("{storage:?}"))
            .unwrap_or_else(|| "untracked".to_string());
        format!("{} [storage={storage}]", self.class.description())
    }
}

/// Exact snapshot of reference representation, referent evidence, and the
/// synchronized storage class. Absence from `classes` means `Value`; absence
/// from `storage_classes` means that the type tracker has no storage evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReferenceFlowState {
    pub(super) classes: BTreeMap<BindingKey, ReferenceClass>,
    pub(super) storage_classes: BTreeMap<BindingKey, BindingStorageClass>,
}

impl ReferenceFlowState {
    pub(super) fn class(&self, key: BindingKey) -> ReferenceClass {
        self.classes
            .get(&key)
            .cloned()
            .unwrap_or(ReferenceClass::Value)
    }

    pub(super) fn evidence(&self, key: BindingKey) -> ReferenceFlowEvidence {
        ReferenceFlowEvidence {
            class: self.class(key),
            storage: self.storage_classes.get(&key).copied(),
        }
    }

    pub(super) fn keys(&self) -> BTreeSet<BindingKey> {
        self.classes
            .keys()
            .chain(self.storage_classes.keys())
            .copied()
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReferenceFlowPredecessor {
    pub(super) label: String,
    pub(super) state: ReferenceFlowState,
    pub(super) reachable: bool,
}

impl ReferenceFlowPredecessor {
    // Slice 1 publishes the exact join vocabulary; control-flow consumers land
    // in the separately owned follow-up slice.
    #[allow(dead_code)]
    pub(crate) fn reachable(label: impl Into<String>, state: ReferenceFlowState) -> Self {
        Self {
            label: label.into(),
            state,
            reachable: true,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn unreachable(label: impl Into<String>, state: ReferenceFlowState) -> Self {
        Self {
            label: label.into(),
            state,
            reachable: false,
        }
    }
}

/// Structural conflict produced before display names or source spans are
/// consulted. `BindingKey` is the authority; names and locations are attached
/// only by the compiler diagnostic adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReferenceFlowConflict {
    pub(super) merge_name: String,
    pub(super) key: BindingKey,
    pub(super) first_label: String,
    pub(super) first: ReferenceFlowEvidence,
    pub(super) second_label: String,
    pub(super) second: ReferenceFlowEvidence,
}

impl ReferenceFlowConflict {
    pub(super) fn new(
        merge_name: impl Into<String>,
        key: BindingKey,
        first_label: impl Into<String>,
        first: ReferenceFlowEvidence,
        second_label: impl Into<String>,
        second: ReferenceFlowEvidence,
    ) -> Self {
        Self {
            merge_name: merge_name.into(),
            key,
            first_label: first_label.into(),
            first,
            second_label: second_label.into(),
            second,
        }
    }
}
