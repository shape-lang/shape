//! Structural identities published by the generated-capture query.

use super::super::CaptureTarget;

/// Slot namespace of the captured binding in its exact owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeneratedCaptureSlot {
    Local(u16),
    ModuleBinding(u16),
}

/// Compiler-issued binding identity used to join capture occurrences.
///
/// Spelling and spans are deliberately absent. A local slot is interpreted
/// inside the structural owner path; a module slot is interpreted inside the
/// source module named by `file_id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum GeneratedCaptureBindingScope {
    Local {
        expansion_fingerprint: (i64, i64),
        owner_path: Vec<String>,
        slot: u16,
    },
    Module {
        slot: u16,
    },
}

/// Opaque compiler-issued identity. Consumers can inspect and compare it but
/// cannot construct one from source spelling or presentation spans.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GeneratedCaptureBindingIdentity {
    scope: GeneratedCaptureBindingScope,
    file_id: u16,
}

impl GeneratedCaptureBindingIdentity {
    pub(super) fn from_capture_target(
        expansion_fingerprint: (i64, i64),
        owner_path: Vec<String>,
        file_id: u16,
        target: CaptureTarget,
    ) -> Self {
        match target {
            CaptureTarget::Local(slot) => Self {
                scope: GeneratedCaptureBindingScope::Local {
                    expansion_fingerprint,
                    owner_path,
                    slot,
                },
                file_id,
            },
            CaptureTarget::ModuleBinding(slot) => Self {
                scope: GeneratedCaptureBindingScope::Module { slot },
                file_id,
            },
        }
    }

    pub fn expansion_fingerprint(&self) -> Option<(i64, i64)> {
        match &self.scope {
            GeneratedCaptureBindingScope::Local {
                expansion_fingerprint,
                ..
            } => Some(*expansion_fingerprint),
            GeneratedCaptureBindingScope::Module { .. } => None,
        }
    }

    pub fn owner_path(&self) -> Option<&[String]> {
        match &self.scope {
            GeneratedCaptureBindingScope::Local { owner_path, .. } => Some(owner_path),
            GeneratedCaptureBindingScope::Module { .. } => None,
        }
    }

    pub fn file_id(&self) -> u16 {
        self.file_id
    }

    pub fn slot(&self) -> GeneratedCaptureSlot {
        match &self.scope {
            GeneratedCaptureBindingScope::Local { slot, .. } => GeneratedCaptureSlot::Local(*slot),
            GeneratedCaptureBindingScope::Module { slot } => {
                GeneratedCaptureSlot::ModuleBinding(*slot)
            }
        }
    }

    /// Stable diagnostic/debug rendering. It is never parsed back as identity.
    pub fn canonical_descriptor(&self) -> String {
        match &self.scope {
            GeneratedCaptureBindingScope::Local {
                expansion_fingerprint: (high, low),
                owner_path,
                slot,
            } => format!(
                "capture:{:016x}{:016x}:file:{}:owner:{}:local:{}",
                *high as u64,
                *low as u64,
                self.file_id,
                owner_path.join("/"),
                slot,
            ),
            GeneratedCaptureBindingScope::Module { slot } => {
                format!("capture:file:{}:module:{slot}", self.file_id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_owner_same_module_slot_has_one_reference_join_identity() {
        let from_first_owner = GeneratedCaptureBindingIdentity::from_capture_target(
            (1, 2),
            vec!["method:first".to_string()],
            7,
            CaptureTarget::ModuleBinding(3),
        );
        let from_second_owner = GeneratedCaptureBindingIdentity::from_capture_target(
            (8, 9),
            vec!["method:second".to_string()],
            7,
            CaptureTarget::ModuleBinding(3),
        );
        assert_eq!(from_first_owner, from_second_owner);
        assert_eq!(from_first_owner.expansion_fingerprint(), None);
        assert_eq!(from_first_owner.owner_path(), None);
        assert_eq!(
            from_first_owner.canonical_descriptor(),
            "capture:file:7:module:3",
        );
    }

    #[test]
    fn local_identity_retains_expansion_and_structural_owner() {
        let first = GeneratedCaptureBindingIdentity::from_capture_target(
            (1, 2),
            vec!["method:first".to_string()],
            0,
            CaptureTarget::Local(3),
        );
        let second = GeneratedCaptureBindingIdentity::from_capture_target(
            (1, 2),
            vec!["method:second".to_string()],
            0,
            CaptureTarget::Local(3),
        );
        assert_ne!(first, second);
    }
}

/// Structural identity of one authored capture occurrence.
///
/// It deliberately excludes both binding and specialization. Sibling closures
/// have distinct full node paths; monomorphized instances of one authored
/// clause share this identity and are aggregated explicitly by the query.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GeneratedCaptureOccurrenceIdentity {
    pub(super) expansion_fingerprint: (i64, i64),
    pub(super) file_id: u16,
    pub(super) capture_node_path: Vec<String>,
    pub(super) descriptor_ordinal: usize,
}

impl GeneratedCaptureOccurrenceIdentity {
    pub fn expansion_fingerprint(&self) -> (i64, i64) {
        self.expansion_fingerprint
    }

    pub fn file_id(&self) -> u16 {
        self.file_id
    }

    pub fn capture_node_path(&self) -> &[String] {
        &self.capture_node_path
    }

    pub fn descriptor_ordinal(&self) -> usize {
        self.descriptor_ordinal
    }

    /// Stable diagnostic/debug rendering. It is never parsed as identity.
    pub fn canonical_descriptor(&self) -> String {
        let (high, low) = self.expansion_fingerprint;
        format!(
            "occurrence:{:016x}{:016x}:file:{}:node:{}:descriptor:{}",
            high as u64,
            low as u64,
            self.file_id,
            self.capture_node_path.join("/"),
            self.descriptor_ordinal,
        )
    }
}
