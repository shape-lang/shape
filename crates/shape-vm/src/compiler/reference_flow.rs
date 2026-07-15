//! Exact, ephemeral flow state for first-class reference-valued bindings.
//!
//! This module does not decide control-flow. It snapshots and restores the
//! compiler's existing reference markers, and provides the one exact join
//! operation control-flow compilers can use. A binding whose runtime
//! representation differs across reachable predecessors is rejected rather
//! than widened to a representation that would miscompile either path.

use std::collections::{BTreeMap, BTreeSet};

use shape_ast::error::{Result, ShapeError};
use shape_value::v2::ConcreteType;

use crate::type_tracking::BindingStorageClass;

use super::BytecodeCompiler;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum BindingKey {
    Local(u16),
    ModuleBinding(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReferenceClass {
    Value,
    SharedReference {
        referent: Option<ConcreteType>,
    },
    ExclusiveReference {
        referent: Option<ConcreteType>,
    },
}

impl ReferenceClass {
    fn description(&self) -> String {
        match self {
            Self::Value => "Value".to_string(),
            Self::SharedReference { referent: None } => "SharedReference<?>".to_string(),
            Self::SharedReference {
                referent: Some(referent),
            } => format!("SharedReference<{referent:?}>"),
            Self::ExclusiveReference { referent: None } => {
                "ExclusiveReference<?>".to_string()
            }
            Self::ExclusiveReference {
                referent: Some(referent),
            } => format!("ExclusiveReference<{referent:?}>"),
        }
    }

    fn is_reference(&self) -> bool {
        !matches!(self, Self::Value)
    }
}

/// Exact snapshot of the existing reference markers and their synchronized
/// storage evidence. Absence from `classes` means [`ReferenceClass::Value`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReferenceFlowState {
    classes: BTreeMap<BindingKey, ReferenceClass>,
    storage_classes: BTreeMap<BindingKey, BindingStorageClass>,
}

impl ReferenceFlowState {
    fn class(&self, key: BindingKey) -> ReferenceClass {
        self.classes
            .get(&key)
            .cloned()
            .unwrap_or(ReferenceClass::Value)
    }

    fn keys(&self) -> impl Iterator<Item = BindingKey> + '_ {
        self.classes.keys().copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReferenceFlowPredecessor {
    label: String,
    state: ReferenceFlowState,
    reachable: bool,
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

impl BytecodeCompiler {
    pub(crate) fn reference_flow_snapshot(&self) -> ReferenceFlowState {
        let mut classes = BTreeMap::new();

        let local_slots: BTreeSet<_> = self
            .reference_value_locals
            .iter()
            .chain(&self.exclusive_reference_value_locals)
            .copied()
            .collect();
        for slot in local_slots {
            let referent = self
                .reference_value_local_referent_concrete_type
                .get(&slot)
                .cloned();
            let class = if self.exclusive_reference_value_locals.contains(&slot) {
                ReferenceClass::ExclusiveReference { referent }
            } else {
                ReferenceClass::SharedReference { referent }
            };
            classes.insert(BindingKey::Local(slot), class);
        }

        let module_slots: BTreeSet<_> = self
            .reference_value_module_bindings
            .iter()
            .chain(&self.exclusive_reference_value_module_bindings)
            .copied()
            .collect();
        for slot in module_slots {
            let referent = self
                .reference_value_module_binding_referent_concrete_type
                .get(&slot)
                .cloned();
            let class = if self
                .exclusive_reference_value_module_bindings
                .contains(&slot)
            {
                ReferenceClass::ExclusiveReference { referent }
            } else {
                ReferenceClass::SharedReference { referent }
            };
            classes.insert(BindingKey::ModuleBinding(slot), class);
        }

        let mut storage_keys = BTreeSet::new();
        storage_keys.extend(classes.keys().copied());
        storage_keys.extend(
            self.locals
                .iter()
                .flat_map(|scope| scope.values().copied())
                .map(BindingKey::Local),
        );
        storage_keys.extend(
            self.module_bindings
                .values()
                .copied()
                .map(BindingKey::ModuleBinding),
        );
        let storage_classes = storage_keys
            .into_iter()
            .filter_map(|key| {
                let storage = match key {
                    BindingKey::Local(slot) => self
                        .type_tracker
                        .get_local_binding_semantics(slot)
                        .map(|semantics| semantics.storage_class),
                    BindingKey::ModuleBinding(slot) => self
                        .type_tracker
                        .get_binding_semantics(slot)
                        .map(|semantics| semantics.storage_class),
                }?;
                Some((key, storage))
            })
            .collect();

        ReferenceFlowState {
            classes,
            storage_classes,
        }
    }

    /// Snapshot the enclosing frame, then start a function frame with no
    /// local reference-valued bindings. Module bindings remain visible while
    /// the function body is compiled and are restored from the snapshot when
    /// that compilation finishes.
    pub(crate) fn enter_function_reference_flow(&mut self) -> ReferenceFlowState {
        let state = self.reference_flow_snapshot();
        self.reference_value_locals.clear();
        self.exclusive_reference_value_locals.clear();
        self.reference_value_local_referent_concrete_type.clear();
        state
    }

    pub(crate) fn restore_reference_flow_snapshot(&mut self, state: &ReferenceFlowState) {
        self.reference_value_locals.clear();
        self.exclusive_reference_value_locals.clear();
        self.reference_value_local_referent_concrete_type.clear();
        self.reference_value_module_bindings.clear();
        self.exclusive_reference_value_module_bindings.clear();
        self.reference_value_module_binding_referent_concrete_type
            .clear();

        for (&key, class) in &state.classes {
            self.install_reference_flow_class(key, class.clone());
        }
        for (&key, &storage) in &state.storage_classes {
            self.set_reference_flow_storage(key, storage);
        }
    }

    // Slice 1 establishes and proves the authority before branch/loop callers
    // are wired by the separately owned control-flow slice.
    #[allow(dead_code)]
    pub(crate) fn join_reference_flow_predecessors(
        &self,
        merge_name: &str,
        predecessors: impl IntoIterator<Item = ReferenceFlowPredecessor>,
    ) -> Result<Option<ReferenceFlowState>> {
        let mut reachable: Vec<_> = predecessors
            .into_iter()
            .filter(|predecessor| predecessor.reachable)
            .collect();
        reachable.sort_by(|left, right| left.label.cmp(&right.label));
        let Some(first) = reachable.first() else {
            return Ok(None);
        };

        let keys: BTreeSet<_> = reachable
            .iter()
            .flat_map(|predecessor| predecessor.state.keys())
            .collect();
        for key in keys {
            let expected = first.state.class(key);
            for predecessor in reachable.iter().skip(1) {
                let actual = predecessor.state.class(key);
                if actual == expected {
                    continue;
                }
                let conflict_kind = if expected.is_reference() == actual.is_reference() {
                    "conflicting"
                } else {
                    "heterogeneous"
                };
                let binding_name = self.reference_flow_binding_name(key);
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "{conflict_kind} reference flow at {merge_name} for binding \
                         '{binding_name}': predecessor '{}' is {}, but predecessor '{}' is {}; \
                         every reachable path must use one exact Value, SharedReference, or \
                         ExclusiveReference representation",
                        first.label,
                        expected.description(),
                        predecessor.label,
                        actual.description(),
                    ),
                    location: None,
                });
            }
        }

        Ok(Some(first.state.clone()))
    }

    pub(crate) fn set_reference_flow_class(
        &mut self,
        key: BindingKey,
        class: ReferenceClass,
    ) {
        self.remove_reference_flow_evidence(key);
        self.install_reference_flow_class(key, class.clone());
        let storage = if class.is_reference() {
            BindingStorageClass::Reference
        } else {
            match key {
                BindingKey::Local(slot) => {
                    self.default_binding_storage_class_for_slot(slot, true)
                }
                BindingKey::ModuleBinding(slot) => {
                    self.default_binding_storage_class_for_slot(slot, false)
                }
            }
        };
        self.set_reference_flow_storage(key, storage);
    }

    pub(crate) fn set_reference_flow_referent(
        &mut self,
        key: BindingKey,
        referent: Option<ConcreteType>,
    ) {
        let is_reference = match key {
            BindingKey::Local(slot) => self.reference_value_locals.contains(&slot),
            BindingKey::ModuleBinding(slot) => {
                self.reference_value_module_bindings.contains(&slot)
            }
        };
        debug_assert!(
            is_reference || referent.is_none(),
            "referent evidence requires a reference-valued binding"
        );
        if !is_reference {
            return;
        }
        match (key, referent) {
            (BindingKey::Local(slot), Some(referent)) => {
                self.reference_value_local_referent_concrete_type
                    .insert(slot, referent);
            }
            (BindingKey::ModuleBinding(slot), Some(referent)) => {
                self.reference_value_module_binding_referent_concrete_type
                    .insert(slot, referent);
            }
            (BindingKey::Local(slot), None) => {
                self.reference_value_local_referent_concrete_type
                    .remove(&slot);
            }
            (BindingKey::ModuleBinding(slot), None) => {
                self.reference_value_module_binding_referent_concrete_type
                    .remove(&slot);
            }
        }
    }

    pub(crate) fn evict_current_scope_reference_flow(&mut self) {
        let slots = self
            .locals
            .last()
            .map(|scope| scope.values().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        self.evict_local_reference_flow_slots(slots);
    }

    fn evict_local_reference_flow_slots(&mut self, slots: impl IntoIterator<Item = u16>) {
        for slot in slots {
            self.remove_reference_flow_evidence(BindingKey::Local(slot));
        }
    }

    fn install_reference_flow_class(&mut self, key: BindingKey, class: ReferenceClass) {
        let (exclusive, referent) = match class {
            ReferenceClass::Value => return,
            ReferenceClass::SharedReference { referent } => (false, referent),
            ReferenceClass::ExclusiveReference { referent } => (true, referent),
        };
        match key {
            BindingKey::Local(slot) => {
                self.reference_value_locals.insert(slot);
                if exclusive {
                    self.exclusive_reference_value_locals.insert(slot);
                }
                if let Some(referent) = referent {
                    self.reference_value_local_referent_concrete_type
                        .insert(slot, referent);
                }
            }
            BindingKey::ModuleBinding(slot) => {
                self.reference_value_module_bindings.insert(slot);
                if exclusive {
                    self.exclusive_reference_value_module_bindings.insert(slot);
                }
                if let Some(referent) = referent {
                    self.reference_value_module_binding_referent_concrete_type
                        .insert(slot, referent);
                }
            }
        }
    }

    fn remove_reference_flow_evidence(&mut self, key: BindingKey) {
        match key {
            BindingKey::Local(slot) => {
                self.reference_value_locals.remove(&slot);
                self.exclusive_reference_value_locals.remove(&slot);
                self.reference_value_local_referent_concrete_type
                    .remove(&slot);
            }
            BindingKey::ModuleBinding(slot) => {
                self.reference_value_module_bindings.remove(&slot);
                self.exclusive_reference_value_module_bindings.remove(&slot);
                self.reference_value_module_binding_referent_concrete_type
                    .remove(&slot);
            }
        }
    }

    fn set_reference_flow_storage(&mut self, key: BindingKey, storage: BindingStorageClass) {
        match key {
            BindingKey::Local(slot) => self
                .type_tracker
                .set_local_binding_storage_class(slot, storage),
            BindingKey::ModuleBinding(slot) => self
                .type_tracker
                .set_binding_storage_class(slot, storage),
        }
    }

    fn reference_flow_binding_name(&self, key: BindingKey) -> String {
        let mut names: Vec<_> = match key {
            BindingKey::Local(slot) => self
                .locals
                .iter()
                .flat_map(|scope| scope.iter())
                .filter_map(|(name, &candidate)| (candidate == slot).then_some(name.clone()))
                .collect(),
            BindingKey::ModuleBinding(slot) => self
                .module_bindings
                .iter()
                .filter_map(|(name, &candidate)| (candidate == slot).then_some(name.clone()))
                .collect(),
        };
        names.sort();
        names.into_iter().next().unwrap_or_else(|| match key {
            BindingKey::Local(slot) => format!("<local:{slot}>"),
            BindingKey::ModuleBinding(slot) => format!("<module-binding:{slot}>"),
        })
    }
}

#[cfg(test)]
#[path = "reference_flow/tests.rs"]
mod tests;
