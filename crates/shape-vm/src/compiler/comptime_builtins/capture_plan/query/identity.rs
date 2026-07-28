//! Structural identities published by the generated-capture query.

use super::super::CaptureBindingLineage;
use shape_ast::ast::{GeneratedExpansionFingerprint, GeneratedNodePath};
use shape_runtime::type_system::GeneratedNodeKey;

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
        expansion_fingerprint: GeneratedExpansionFingerprint,
        owner_path: GeneratedNodePath,
        slot: u16,
    },
    Module {
        slot: u16,
    },
}

/// Opaque, compiler-session-scoped join identity. Consumers can inspect and
/// compare it inside the [`super::GeneratedCaptureQuery`] that issued it, but
/// must not cache it across compilations or treat it as a persistent semantic
/// identity: `file_id` and binding slots are order-assigned session coordinates.
/// Local identities additionally retain the compiler-issued declaration-path
/// labels carried by structural provenance. Capture/binding spelling, source
/// spans, and owner-display prose do not mint this identity.
///
/// Serialization is deliberately unsupported so this query-local carrier
/// cannot silently become a cross-session cache key:
///
/// ```compile_fail
/// use shape_vm::compiler::GeneratedCaptureBindingIdentity;
///
/// fn require_serializable<T: serde::Serialize>() {}
/// require_serializable::<GeneratedCaptureBindingIdentity>();
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GeneratedCaptureBindingIdentity {
    scope: GeneratedCaptureBindingScope,
    file_id: u16,
}

impl GeneratedCaptureBindingIdentity {
    pub(super) fn from_binding_lineage(lineage: &CaptureBindingLineage) -> Self {
        match lineage {
            CaptureBindingLineage::Local {
                expansion_fingerprint,
                binding_owner_path,
                file_id,
                slot,
            } => Self {
                scope: GeneratedCaptureBindingScope::Local {
                    expansion_fingerprint: *expansion_fingerprint,
                    owner_path: binding_owner_path.clone(),
                    slot: *slot,
                },
                file_id: *file_id,
            },
            CaptureBindingLineage::ModuleBinding { file_id, slot } => Self {
                scope: GeneratedCaptureBindingScope::Module { slot: *slot },
                file_id: *file_id,
            },
        }
    }

    pub fn expansion_fingerprint(&self) -> Option<(i64, i64)> {
        match &self.scope {
            GeneratedCaptureBindingScope::Local {
                expansion_fingerprint,
                ..
            } => Some(expansion_fingerprint.components()),
            GeneratedCaptureBindingScope::Module { .. } => None,
        }
    }

    pub fn owner_path(&self) -> Option<&[String]> {
        match &self.scope {
            GeneratedCaptureBindingScope::Local { owner_path, .. } => Some(owner_path.segments()),
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

    /// Deterministic diagnostic/debug rendering within one compiler session.
    /// It is never parsed back or persisted as cross-session identity.
    pub fn canonical_descriptor(&self) -> String {
        match &self.scope {
            GeneratedCaptureBindingScope::Local {
                expansion_fingerprint,
                owner_path,
                slot,
            } => {
                let (high, low) = expansion_fingerprint.components();
                format!(
                    "capture:{:016x}{:016x}:file:{}:owner:{}:local:{}",
                    high as u64,
                    low as u64,
                    self.file_id,
                    owner_path.render(),
                    slot,
                )
            }
            GeneratedCaptureBindingScope::Module { slot } => {
                format!("capture:file:{}:module:{slot}", self.file_id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::GeneratedNodeOrigin;

    #[test]
    fn cross_owner_same_module_slot_has_one_reference_join_identity() {
        let lineage = CaptureBindingLineage::ModuleBinding {
            file_id: 7,
            slot: 3,
        };
        let from_first_owner = GeneratedCaptureBindingIdentity::from_binding_lineage(&lineage);
        let from_second_owner = GeneratedCaptureBindingIdentity::from_binding_lineage(&lineage);
        assert_eq!(from_first_owner, from_second_owner);
        assert_eq!(from_first_owner.expansion_fingerprint(), None);
        assert_eq!(from_first_owner.owner_path(), None);
        assert_eq!(
            from_first_owner.canonical_descriptor(),
            "capture:file:7:module:3",
        );
    }

    #[test]
    fn local_identity_retains_typed_expansion_and_structural_owner() {
        let first =
            GeneratedCaptureBindingIdentity::from_binding_lineage(&CaptureBindingLineage::Local {
                expansion_fingerprint: GeneratedExpansionFingerprint::from_components(1, 2),
                binding_owner_path: GeneratedNodePath::decl_root("method:first"),
                file_id: 0,
                slot: 3,
            });
        let second =
            GeneratedCaptureBindingIdentity::from_binding_lineage(&CaptureBindingLineage::Local {
                expansion_fingerprint: GeneratedExpansionFingerprint::from_components(1, 2),
                binding_owner_path: GeneratedNodePath::decl_root("method:second"),
                file_id: 0,
                slot: 3,
            });
        let different_expansion =
            GeneratedCaptureBindingIdentity::from_binding_lineage(&CaptureBindingLineage::Local {
                expansion_fingerprint: GeneratedExpansionFingerprint::from_components(2, 1),
                binding_owner_path: GeneratedNodePath::decl_root("method:first"),
                file_id: 0,
                slot: 3,
            });
        assert_ne!(first, second);
        assert_ne!(first, different_expansion);
        assert_eq!(first.expansion_fingerprint(), Some((1, 2)));
        assert_eq!(first.owner_path(), Some(&["method:first".to_string()][..]));
        assert_eq!(
            first.canonical_descriptor(),
            "capture:00000000000000010000000000000002:file:0:owner:method:first:local:3",
        );
    }

    #[test]
    fn binding_identity_exposes_its_session_assigned_coordinate_boundary() {
        let baseline = GeneratedCaptureBindingIdentity::from_binding_lineage(
            &CaptureBindingLineage::ModuleBinding {
                file_id: 7,
                slot: 3,
            },
        );
        let reordered_file = GeneratedCaptureBindingIdentity::from_binding_lineage(
            &CaptureBindingLineage::ModuleBinding {
                file_id: 8,
                slot: 3,
            },
        );
        let reordered_slot = GeneratedCaptureBindingIdentity::from_binding_lineage(
            &CaptureBindingLineage::ModuleBinding {
                file_id: 7,
                slot: 4,
            },
        );

        assert_ne!(baseline, reordered_file);
        assert_ne!(baseline, reordered_slot);
    }

    #[test]
    fn occurrence_identity_ignores_order_assigned_source_file_and_span() {
        let first_origin = decoded_origin(7, 10, 20, "first presentation");
        let second_origin = decoded_origin(99, 80, 90, "second presentation");
        let first = GeneratedCaptureOccurrenceIdentity {
            node: GeneratedNodeKey::from_origin(&first_origin),
            descriptor_ordinal: 3,
        };
        let second = GeneratedCaptureOccurrenceIdentity {
            node: GeneratedNodeKey::from_origin(&second_origin),
            descriptor_ordinal: 3,
        };

        assert_eq!(first, second);
        assert_eq!(
            first.canonical_descriptor(),
            "occurrence:000000000000000b000000000000001d:node:method:read/closure:0:descriptor:3",
        );
    }

    fn decoded_origin(
        anchor_file_id: u16,
        start: usize,
        end: usize,
        owner_display: &str,
    ) -> GeneratedNodeOrigin {
        serde_json::from_value(serde_json::json!({
            "expansion_high": 11,
            "expansion_low": 29,
            "node_path": ["method:read", "closure:0"],
            "anchor_file_id": anchor_file_id,
            "anchor_span": { "start": start, "end": end },
            "owner_display": owner_display,
        }))
        .expect("serialized provenance data decodes without compiler authority")
    }
}

/// Structural identity of one authored capture occurrence.
///
/// It deliberately excludes both binding and specialization. Sibling closures
/// have distinct full node paths; monomorphized instances of one authored
/// clause share this identity and are aggregated explicitly by the query. The
/// node path can contain compiler-issued declaration labels such as
/// `extend:Type` and `method:name`; capture/binding spelling, source spans,
/// owner-display prose, and traversal/file order remain excluded.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GeneratedCaptureOccurrenceIdentity {
    pub(super) node: GeneratedNodeKey,
    pub(super) descriptor_ordinal: usize,
}

impl GeneratedCaptureOccurrenceIdentity {
    pub fn expansion_fingerprint(&self) -> (i64, i64) {
        self.node.expansion_fingerprint()
    }

    pub fn capture_node_path(&self) -> &[String] {
        self.node.node_path()
    }

    pub fn descriptor_ordinal(&self) -> usize {
        self.descriptor_ordinal
    }

    /// Stable diagnostic/debug rendering. It is never parsed as identity.
    pub fn canonical_descriptor(&self) -> String {
        let (high, low) = self.node.expansion_fingerprint();
        format!(
            "occurrence:{:016x}{:016x}:node:{}:descriptor:{}",
            high as u64,
            low as u64,
            self.node.node_path().join("/"),
            self.descriptor_ordinal,
        )
    }
}
