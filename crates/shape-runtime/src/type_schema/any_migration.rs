//! The one module that can construct `FieldType::Any` — #235 day-one gate.
//!
//! Owner ruling 2026-08-02 (issue #235, grill R-G4 in
//! `docs/program/adr020/grill-rulings-2026-08-02.md`) deletes
//! `FieldType::Any` outright. The architecture statement behind the
//! deletion is:
//!
//! > Unknown is representable only in the inference tier
//! > (`Type::Variable` / `ProofGap`). The schema tier has no unknown
//! > state — a schema can only be minted from resolved types.
//!
//! Deleting a variant that ~75 sites construct takes several stages, and
//! the failure mode of a staged deletion is that new construction sites
//! appear faster than old ones are retired. This module closes that off
//! mechanically, using the same shape as `ProofGap` in
//! `crates/shape-vm/src/compiler/type_tracking.rs`: the `Any` variant
//! carries an [`AnyToken`], `AnyToken`'s only field is private to this
//! module, and therefore **`FieldType::Any(..)` cannot be written
//! anywhere else in the workspace**. It is a compile error, not a lint.
//!
//! Every surviving construction site routes through one of the named
//! constructors below. Each name states which deletion class the site
//! belongs to, so a reviewer can tell at a glance whether a new caller is
//! a legitimate member of a class that has not been retired yet, or a new
//! defection. The `any-typed-carriers` shrink-only ratchet (CHECK 15,
//! `scripts/lib/adr011-012-legacy-sets.mjs`) counts the total and refuses
//! growth.
//!
//! Classes, per the R-G4 staging plan:
//!
//! - **A** — inference gaps where the resolved type was computed and then
//!   discarded. Retired in stage 1; no constructor exists for it.
//! - **B** — genuinely heterogeneous stdlib carriers.
//!   [`heterogeneous_stdlib_carrier`]. Stage 2 moves these to the
//!   value-tier kind track (ADR-006 §2.7.7), never a schema claim.
//! - **C** — `array_field(_, Any)` one-liners with the element type
//!   already proven at the producer. Retired in stage 1.
//! - **D** — provably-empty `bounds` arrays. [`bounds_array_element`].
//!   Resolves via the `never` element type in #266.
//! - **E** — enum `__payload_N` slots. [`enum_payload_slot`]. Own design
//!   ticket, #267.
//! - **F** — runtime schema synthesis. Retired in stage 1 (the whole
//!   auto-registration path is deleted).
//!
//! Two constructors below are not deletion classes but residue that stage
//! 1 does not touch: [`unprojectable_annotation`] (the
//! `type_annotation_to_field_type` tail) and [`field_tag_roundtrip`] (the
//! runtime `FIELD_TAG_*` decode). They are named separately so they are
//! not mistaken for class members.

use crate::type_schema::field_types::FieldType;

/// Capability token proving the holder is inside the `#235` migration
/// module. Construction of the private field is module-private, so
/// `FieldType::Any(AnyToken(..))` is unwriteable outside this file and
/// there is no public path to mint one.
///
/// Deliberately NOT `Default`, NOT `From<()>`, and with no public
/// constructor — adding any of those reopens the hole this type exists to
/// close.
#[derive(Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AnyToken(PrivateWitness);

#[derive(Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
struct PrivateWitness;

/// The single mint. Private to this module by construction.
fn mint() -> FieldType {
    FieldType::Any(AnyToken(PrivateWitness))
}

/// Reconstruct an `Any` while cloning a `FieldType` that already holds one.
///
/// `FieldType`'s hand-written `Clone` (in `field_types.rs`) calls this. It is
/// not a new carrier — the caller already had an `Any` in hand — but it must
/// still go through the mint because [`AnyToken`] is deliberately neither
/// `Clone` nor `Copy`, precisely so that no site outside this module can
/// launder a token out of an existing value.
pub(crate) fn clone_of_existing() -> FieldType {
    mint()
}

/// **Class A** — a site where the resolved field type was computed and
/// then thrown away in favour of an `Any`-uniform layout.
///
/// Stage 1 of #235 retires every caller of this function and deletes it.
/// If you are reading this and it still exists, stage 1 did not finish.
pub fn class_a_inference_gap() -> FieldType {
    mint()
}

/// **Class C** — an `array_field(name, Any)` one-liner whose element type
/// is proven at the producer a few lines away.
///
/// Stage 1 of #235 retires every caller of this function and deletes it.
pub fn class_c_array_field_element() -> FieldType {
    mint()
}

/// **Class F** — runtime schema synthesis: minting an all-`Any` schema
/// from a bare field-name list at execution time, for a field set nobody
/// declared.
///
/// Stage 1 of #235 deletes the auto-registration path entirely; a schema
/// that did not exist at compile time is a producer-side bug, not
/// something to invent at runtime. The JIT already refuses this shape by
/// name (`mir_compiler/statements.rs`).
pub fn class_f_runtime_synthesis() -> FieldType {
    mint()
}

/// **Class B** — a stdlib carrier whose field is genuinely heterogeneous
/// across instances (VM-state introspection payloads, `__Option` /
/// `__Result` payload slots).
///
/// Stage 2 of #235 moves these to the value-tier `NativeKind` track per
/// ADR-006 §2.7.7, which is the legitimate home for per-element dynamism.
/// A schema claim of `Any` is not.
pub fn heterogeneous_stdlib_carrier() -> FieldType {
    mint()
}

/// **Class D** — the element type of a `bounds` array that is provably
/// empty at every construction site.
///
/// Resolves via the `never` element type ruled in #266 (grill R-G1); this
/// constructor disappears with that ticket.
pub fn bounds_array_element() -> FieldType {
    mint()
}

/// **Class E** — an enum variant's `__payload_N` slot.
///
/// The compiler already carries a per-variant kind track; #267 designs the
/// per-variant typed payload layout that materializes it into the schema.
pub fn enum_payload_slot() -> FieldType {
    mint()
}

/// Residue of `BytecodeCompiler::type_annotation_to_field_type`
/// (`crates/shape-vm/src/compiler/helpers.rs`) — annotations that have no
/// `FieldType` projection at all (`Borrow`, `Tuple`, `Function`, `Union`,
/// `Intersection`, `Dyn`, `Existential`, `Void`, `Never`, `Null`,
/// `Undefined`).
///
/// Not a #235 class: the correct fix is for that function to return
/// `Option<FieldType>`/`Result` and for callers to surface, rather than to
/// project an unprojectable annotation onto a schema type. Out of stage-1
/// scope.
pub fn unprojectable_annotation() -> FieldType {
    mint()
}

/// The inner type of an `Option` minted from a bare `None` literal, before
/// bidirectional narrowing supplies `T`.
///
/// The outer discriminator is concrete; only the inner slot is unresolved.
/// Not a #235 class — it is an inference-tier unknown that currently leaks
/// one level into the schema tier.
pub fn unresolved_option_inner() -> FieldType {
    mint()
}

/// Runtime `FIELD_TAG_*` byte → `FieldType` decode
/// (`crates/shape-vm/src/executor/typed_object_ops.rs`). The tag byte is
/// one byte wide and cannot carry an element type, so the decode
/// reconstructs container types with an unresolved element.
///
/// Not a #235 class: this is the persisted-tag width limit, tracked with
/// the snapshot format rather than with the variant deletion.
pub fn field_tag_roundtrip() -> FieldType {
    mint()
}

/// Test fixtures that assert on `Any`-carrying schemas.
///
/// Production code must never call this. It exists so that the E0900
/// verification tests, the schema-layout tests, and the field-tag
/// round-trip tests can keep building `Any`-bearing schemas while the
/// variant is being deleted; those tests die with the variant.
pub fn test_fixture() -> FieldType {
    mint()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_constructor_yields_the_same_variant() {
        for ft in [
            heterogeneous_stdlib_carrier(),
            bounds_array_element(),
            enum_payload_slot(),
            unprojectable_annotation(),
            unresolved_option_inner(),
            field_tag_roundtrip(),
            test_fixture(),
        ] {
            assert!(matches!(ft, FieldType::Any(_)));
        }
    }

    #[test]
    fn tokens_compare_equal_so_schema_interning_is_unaffected() {
        // Schema content-interning (`register_inline_object_schema_typed`)
        // hashes field types. The token must not make two `Any` fields
        // compare unequal, or interning would silently stop deduplicating.
        assert_eq!(heterogeneous_stdlib_carrier(), enum_payload_slot());
    }
}
