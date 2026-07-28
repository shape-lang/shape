//! Deriving effect rows for calls into the stdlib (ADR-014 §8.4, caveat one).
//!
//! # The caveat this closes
//!
//! `capability_tags::required_permissions` is a string-keyed table over about
//! six stdlib module paths whose every arm — and the whole-table fallthrough —
//! ends in `PermissionSet::pure()`. Read as a permission gate that is
//! defensible: an unlisted call gates on nothing because it is believed to do
//! nothing. Read as an *effect row* it is a purity proof manufactured out of a
//! missing table entry, and ADR-014 §8.4 names it as a soundness caveat this
//! work must close rather than inherit.
//!
//! # The choice made here
//!
//! Of the two dispositions the ticket allows — surface-and-stop, or explicit
//! conservative rows — this module takes **conservative rows**.
//!
//! - A module path the table knows, at a function the module's arm names,
//!   derives a closed row from that arm's permissions. This is a real
//!   derivation: the table entry is positive evidence.
//! - Anything else — an unknown module, or a known module at a function its
//!   arm does not name — derives [`ClosedEffectRow::conservative_top`], every
//!   atom legal at the stage. Not `{}`.
//!
//! Surface-and-stop was rejected because it would make every call to an
//! unlisted stdlib function a hard compile error the moment any boundary
//! declares a row, which is most of the stdlib; the conservative row keeps
//! such a call checkable and simply refuses to let it satisfy a narrow
//! boundary. The cost is honest and visible: a caller that wants a narrow row
//! over an unlisted callee must widen its declaration or the callee must be
//! listed. It cannot silently pass.
//!
//! The table's arms are consulted through the existing
//! `required_permissions` entry point rather than copied, so there is one
//! source of the mapping. What this module adds is the distinction the table
//! cannot make: *listed and empty* (genuinely pure) versus *not listed*
//! (unknown).

use shape_abi_v1::Permission;

use super::{ClosedEffectRow, EffectAtom, EffectRow, EffectStage, OperationalEffectId};
use crate::stdlib::capability_tags::required_permissions;

/// Module paths whose per-function arms are real evidence. A path outside this
/// list has no entry at all, so its absence from `required_permissions` says
/// nothing about what it does.
const DERIVABLE_MODULES: [&str; 11] = [
    "std::core::io",
    "std::core::file",
    "std::core::http",
    "std::core::env",
    "std::core::time",
    "std::core::csv",
    "std::core::json",
    "std::core::crypto",
    "std::core::testing",
    "std::core::regex",
    "std::core::math",
];

/// Functions the table names explicitly, per module. A module arm ends in a
/// `_ => pure()` fallthrough, so only these names carry evidence; anything
/// else in the same module lands on the fallthrough and is unknown, not pure.
fn named_functions(module: &str) -> &'static [&'static str] {
    match module {
        "std::core::io" => &[
            "open",
            "read_file",
            "write_file",
            "tcp_connect",
            "listen",
            "spawn",
            "exec",
        ],
        "std::core::file" => &[
            "read_text",
            "read_lines",
            "read_bytes",
            "write_text",
            "write_bytes",
            "append",
        ],
        "std::core::http" => &["get", "post", "put", "delete"],
        "std::core::env" => &["get", "has", "all", "args", "cwd"],
        "std::core::time" => &["millis"],
        "std::core::csv" => &[
            "read_file",
            "parse",
            "parse_records",
            "stringify",
            "stringify_records",
            "is_valid",
        ],
        // Whole-module pure-computation arms: the module itself is the
        // evidence, so every function in it is covered.
        "std::core::json" | "std::core::crypto" | "std::core::testing" | "std::core::regex"
        | "std::core::math" => &["*"],
        _ => &[],
    }
}

/// Map one host permission to the operational effect it evidences.
///
/// Only the operational category maps. Scoped grants and execution
/// constraints are different facts (ADR-014 §2) and are deliberately not
/// effects — a `FsScoped` grant narrows an authority, it does not describe
/// behavior.
fn permission_as_effect(permission: Permission) -> Option<OperationalEffectId> {
    match permission {
        Permission::FsRead => Some(OperationalEffectId::FsRead),
        Permission::FsWrite => Some(OperationalEffectId::FsWrite),
        Permission::NetConnect => Some(OperationalEffectId::NetConnect),
        Permission::NetListen => Some(OperationalEffectId::NetListen),
        Permission::Process => Some(OperationalEffectId::Process),
        Permission::Env => Some(OperationalEffectId::Env),
        Permission::Time => Some(OperationalEffectId::Time),
        Permission::Random => Some(OperationalEffectId::Random),
        _ => None,
    }
}

/// True iff the permission table carries positive evidence about this call.
pub fn is_derivable(module: &str, function: &str) -> bool {
    if !DERIVABLE_MODULES.contains(&module) {
        return false;
    }
    let named = named_functions(module);
    named == ["*"] || named.contains(&function)
}

/// Derive the row for a stdlib call.
///
/// Returns a closed row either way; the difference is what it means. A
/// derivable call yields its evidenced row, which may legitimately be `{}`.
/// A non-derivable call yields the conservative top row, which is the honest
/// statement "this could do anything in the catalog" and satisfies only a
/// boundary that permits everything.
pub fn effect_row_for_stdlib_call(module: &str, function: &str, stage: EffectStage) -> EffectRow {
    if !is_derivable(module, function) {
        return EffectRow::Closed(ClosedEffectRow::conservative_top(stage));
    }
    let mut row = ClosedEffectRow::pure(stage);
    for permission in required_permissions(module, function).iter() {
        if let Some(id) = permission_as_effect(*permission) {
            // `insert` only rejects a stage-illegal atom, and every
            // operational atom is legal at both stages.
            let _ = row.insert(EffectAtom::Operation(id));
        }
    }
    EffectRow::Closed(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_of(module: &str, function: &str) -> ClosedEffectRow {
        effect_row_for_stdlib_call(module, function, EffectStage::Runtime)
            .prove_closed()
            .expect("stdlib derivation always yields a closed row")
            .clone()
    }

    #[test]
    fn a_listed_call_derives_its_evidenced_row() {
        assert_eq!(row_of("std::core::file", "read_text").render(), "{FsRead}");
        assert_eq!(row_of("std::core::http", "get").render(), "{NetConnect}");
        assert_eq!(row_of("std::core::io", "spawn").render(), "{Process}");
    }

    #[test]
    fn a_whole_module_pure_arm_really_is_pure() {
        // These modules are pure-computation by module, so the evidence
        // covers every function in them and `{}` is a genuine derivation.
        assert!(row_of("std::core::math", "sqrt").is_pure());
        assert!(row_of("std::core::json", "parse").is_pure());
    }

    #[test]
    fn an_unknown_module_is_not_proven_pure() {
        // THE CAVEAT. `required_permissions` answers `pure()` here, and this
        // is the assertion that stops that answer becoming a purity proof.
        assert!(
            required_permissions("std::core::database", "query").is_empty(),
            "precondition: the permission table falls through to pure() here"
        );
        let row = row_of("std::core::database", "query");
        assert!(
            !row.is_pure(),
            "absence from the permission table must not derive as `{{}}`"
        );
        assert_eq!(
            row,
            ClosedEffectRow::conservative_top(EffectStage::Runtime),
            "an unlisted call derives the conservative top row"
        );
    }

    #[test]
    fn an_unlisted_function_of_a_known_module_is_not_proven_pure() {
        // `std::core::file` is a known module, but its arm does not name
        // `delete_tree`, so the call lands on the arm's `_ => pure()`
        // fallthrough. Same hole, one level in.
        assert!(required_permissions("std::core::file", "delete_tree").is_empty());
        assert!(!row_of("std::core::file", "delete_tree").is_pure());
    }

    #[test]
    fn a_conservative_row_cannot_satisfy_a_narrow_boundary() {
        // The point of the conservative row: it stays checkable but it can
        // never pass for something narrow.
        let unknown = row_of("std::core::database", "query");
        let narrow = ClosedEffectRow::from_atoms(
            EffectStage::Runtime,
            [EffectAtom::Operation(OperationalEffectId::FsRead)],
        )
        .unwrap();
        assert!(!unknown.is_subset_of(&narrow).unwrap());
    }

    #[test]
    fn scoped_grants_and_constraints_are_not_effects() {
        // ADR-014 §2: a scope narrows an authority and a constraint restricts
        // the evaluator. Neither describes behaviour, so neither becomes an
        // atom.
        assert!(permission_as_effect(Permission::FsScoped).is_none());
        assert!(permission_as_effect(Permission::NetScoped).is_none());
        assert!(permission_as_effect(Permission::Deterministic).is_none());
        assert!(permission_as_effect(Permission::MemLimited).is_none());
    }

    #[test]
    fn derivation_is_deterministic() {
        for _ in 0..8 {
            assert_eq!(row_of("std::core::io", "open").render(), "{FsRead}");
            assert_eq!(
                row_of("std::core::database", "query").canonical_form(),
                ClosedEffectRow::conservative_top(EffectStage::Runtime).canonical_form()
            );
        }
    }
}
