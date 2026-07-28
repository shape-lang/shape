//! ADR-009 (ticket B2, slice S3): opaque `TraitRef` / `ImplRef` carriers.
//!
//! Compiler-issued trait identities and implementation evidence ride in the
//! reserved, SOH-prefixed schemas `"\u{1}comptime:TraitRef"` /
//! `"\u{1}comptime:ImplRef"` (`builtin_schemas.rs`), carrying ONLY canonical
//! identity halves — never a name, so no name-based lookup survives into a
//! carrier (Dec 49).
//!
//! ## R7 (forged evidence) is blocked STRUCTURALLY
//!
//! The decode in this module is schema-name-checked against the reserved
//! SOH-prefixed schema names, which cannot be spelled in a Shape identifier
//! or string-constructed into a nominal type: a hand-built typed object, a
//! legacy descriptor, or any other lookalike can never pass the schema-name
//! check. Additionally, decoded identities are re-validated against the
//! per-compilation-unit semantic freeze (`is_frozen_trait_identity` /
//! `impl_evidence_of`), so even a bit-identical carrier holding identity
//! halves the freeze never issued is a named rejection. S5's R7 test
//! asserts this existing invariant.
//!
//! ## ImplRef ties evidence to the exact pair and the exact impl
//!
//! The `ImplRef` carrier holds all three canonical identities — trait, type,
//! and the impl's own `impl:{trait}:{type}:{impl_name_or_default}` hash — so
//! evidence certifies exactly one `(type, trait)` pair and exactly one
//! (possibly named) impl, and the impl's canonical identity is transportable
//! into generated-artifact descriptor fingerprints (Dec 49; the S4 proof
//! asserts the fingerprint entry end-to-end).

use super::semantic_freeze::{FreezeOverlay, FrozenImplEvidence};
use super::type_reflection::{FrozenTypeIdentity, typed_slot_into_heap_value};
use shape_runtime::module_exports::{ModuleExports, ModuleParam};
use shape_runtime::type_schema::TypeSchema;
use shape_runtime::type_schema::builtin_schemas::{
    COMPTIME_FROZEN_IMPL_REF_SCHEMA, COMPTIME_FROZEN_TRAIT_REF_SCHEMA,
    COMPTIME_FROZEN_TYPE_REF_SCHEMA,
};
use shape_runtime::type_schema::typed_object_for_named_schema;
use shape_runtime::typed_module_exports::{
    ConcreteReturn, ConcreteType, TypedReturn, register_typed_function,
};
use shape_value::heap_value::{HeapKind, HeapValue, TypedObjectStorage};
use shape_value::{KindedSlot, NativeKind};
use std::sync::Arc;

/// Rejection R7 (trait half): a TraitRef identity the semantic freeze never
/// issued — whether at carrier construction or at decode — is a named
/// compile-stage error, never a usable TraitRef.
pub(crate) const FORGED_TRAIT_REF_DIAGNOSTIC: &str = "TraitRef identity was not issued by the semantic freeze: only trait_ref \
     over a trait declared before the registration-complete barrier issues \
     TraitRef values";

/// Rejection R7 (evidence half): implementation evidence cannot be forged —
/// an ImplRef whose identities the semantic freeze never issued as evidence
/// is a named compile-stage error.
#[cfg(test)]
pub(crate) const FORGED_IMPL_EVIDENCE_DIAGNOSTIC: &str = "ImplRef evidence was not issued by the semantic freeze: implementation \
     evidence cannot be forged — only find_impl over frozen barrier truth \
     issues ImplRef values";

/// Rejection R2 shape at the intrinsic tier (slice S4; S5 pins the full
/// checker-side matrix): the rewrite lowered a `trait_ref(Name)` whose bare
/// identifier did not resolve to a trait the freeze declared — a value type,
/// an unknown name, or anything else that is not barrier-registered trait
/// truth. Only a declared trait forms a `TraitRef`.
pub(crate) const TRAIT_REF_NOT_A_TRAIT_DIAGNOSTIC: &str = "trait_ref expects a trait declared before the registration-complete \
     barrier: only a declared trait forms a TraitRef (a value type never \
     does)";

/// Rejection R1 (slice S5, Dec 49): the mirror direction of R2 — a TRAIT
/// name where a value type is required. `type_ref(TraitName)` transports the
/// frozen trait identity (identity-literal transport, no strings) and the
/// TypeRef carrier builder answers with this named rejection instead of the
/// generic unknown-identity error (which genuinely-unknown names keep, A1
/// row 2).
pub(crate) const TRAIT_NOT_A_VALUE_TYPE_DIAGNOSTIC: &str = "traits are not value types: type_ref cannot form a TypeRef over a trait \
     — a trait's distinct opaque identity is formed by trait_ref(TraitName)";

/// Ruled S4 stance (surface-and-stop, S6 logs it): `find_impl` resolves the
/// pair's DEFAULT implementation. A pair implemented ONLY through named
/// impls has real evidence but no canonical default — fabricating one would
/// be partial evidence (R9), and silently answering `None` would masquerade
/// an implemented pair as unimplemented. Named-impl selection is future
/// surface; until then this is a named stop.
pub(crate) const FIND_IMPL_NAMED_IMPLS_ONLY_DIAGNOSTIC: &str = "find_impl resolves the pair's default implementation, but this (type, \
     trait) pair is implemented only through named impls; selecting a named \
     implementation as evidence is not part of the find_impl surface";

/// The three canonical identities an `ImplRef` carrier certifies: the exact
/// `(trait, type)` pair plus the exact (possibly named) impl.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImplRefIdentities {
    pub(crate) trait_identity: FrozenTypeIdentity,
    pub(crate) type_identity: FrozenTypeIdentity,
    pub(crate) impl_identity: FrozenTypeIdentity,
}

/// Build the opaque `TraitRef` carrier for a freeze-issued trait identity.
/// An identity the freeze never issued is the named R7 rejection.
pub(crate) fn build_frozen_trait_ref_heap_value(
    identity: FrozenTypeIdentity,
    freeze: &FreezeOverlay,
) -> Result<HeapValue, String> {
    if !freeze.is_frozen_trait_identity(identity) {
        return Err(FORGED_TRAIT_REF_DIAGNOSTIC.to_string());
    }
    typed_slot_into_heap_value(typed_object_for_named_schema(
        COMPTIME_FROZEN_TRAIT_REF_SCHEMA,
        &[
            ("identity_high", KindedSlot::from_int(identity.high)),
            ("identity_low", KindedSlot::from_int(identity.low)),
        ],
    ))
}

/// Schema-name-checked opaque decode of a `TraitRef` carrier back to its
/// freeze-issued trait identity (see the module doc: this decode is what
/// blocks R7 forged evidence structurally).
pub(crate) fn frozen_trait_identity_from_ref(
    slot: &KindedSlot,
    freeze: &FreezeOverlay,
) -> Result<FrozenTypeIdentity, String> {
    let (schema, storage) = reserved_carrier_storage(slot, COMPTIME_FROZEN_TRAIT_REF_SCHEMA)?;
    let identity = identity_halves(&schema, storage, "identity_high", "identity_low")?;
    if !freeze.is_frozen_trait_identity(identity) {
        return Err(FORGED_TRAIT_REF_DIAGNOSTIC.to_string());
    }
    Ok(identity)
}

/// Build the opaque `ImplRef` carrier from one frozen impl fact. The only
/// in-tree producers of [`FrozenImplEvidence`] are the semantic freeze's
/// barrier-time inputs, so every carrier is compiler-issued by construction.
pub(crate) fn build_impl_ref_heap_value(
    evidence: &FrozenImplEvidence,
) -> Result<HeapValue, String> {
    typed_slot_into_heap_value(typed_object_for_named_schema(
        COMPTIME_FROZEN_IMPL_REF_SCHEMA,
        &[
            (
                "trait_identity_high",
                KindedSlot::from_int(evidence.trait_identity.high),
            ),
            (
                "trait_identity_low",
                KindedSlot::from_int(evidence.trait_identity.low),
            ),
            (
                "type_identity_high",
                KindedSlot::from_int(evidence.type_identity.high),
            ),
            (
                "type_identity_low",
                KindedSlot::from_int(evidence.type_identity.low),
            ),
            (
                "impl_identity_high",
                KindedSlot::from_int(evidence.identity.high),
            ),
            (
                "impl_identity_low",
                KindedSlot::from_int(evidence.identity.low),
            ),
        ],
    ))
}

/// Schema-name-checked opaque decode of an `ImplRef` carrier back to its
/// three freeze-issued identities. Identities that do not correspond to
/// evidence the freeze actually issued are the named R7 rejection.
#[cfg(test)]
pub(crate) fn impl_ref_identities_from_evidence_slot(
    slot: &KindedSlot,
    freeze: &FreezeOverlay,
) -> Result<ImplRefIdentities, String> {
    let (schema, storage) = reserved_carrier_storage(slot, COMPTIME_FROZEN_IMPL_REF_SCHEMA)?;
    let identities = ImplRefIdentities {
        trait_identity: identity_halves(
            &schema,
            storage,
            "trait_identity_high",
            "trait_identity_low",
        )?,
        type_identity: identity_halves(
            &schema,
            storage,
            "type_identity_high",
            "type_identity_low",
        )?,
        impl_identity: identity_halves(
            &schema,
            storage,
            "impl_identity_high",
            "impl_identity_low",
        )?,
    };
    // Re-validate against the freeze: the decoded triple must be evidence
    // the freeze actually issued for exactly this pair (a stance
    // surface-and-stop like blanket/widening means no frozen evidence
    // exists, so a carrier claiming it is forged too).
    let issued = matches!(
        freeze.impl_evidence_of(identities.trait_identity, identities.type_identity),
        Ok(Some(set))
            if set.default_impl().map(|e| e.identity) == Some(identities.impl_identity)
                || set
                    .named_impls()
                    .iter()
                    .any(|e| e.identity == identities.impl_identity)
    );
    if !issued {
        return Err(FORGED_IMPL_EVIDENCE_DIAGNOSTIC.to_string());
    }
    Ok(identities)
}

/// Schema-name-checked opaque decode of a `TypeRef` carrier back to its
/// freeze-issued TYPE identity, for `find_impl`'s first argument. The
/// identity is re-validated against the freeze (`category_of` errs on any
/// identity the freeze never issued — the same R7 posture as the trait and
/// evidence decodes).
pub(crate) fn frozen_type_identity_from_type_ref(
    slot: &KindedSlot,
    freeze: &FreezeOverlay,
) -> Result<FrozenTypeIdentity, String> {
    let (schema, storage) = reserved_carrier_storage(slot, COMPTIME_FROZEN_TYPE_REF_SCHEMA)?;
    let identity = identity_halves(&schema, storage, "identity_high", "identity_low")?;
    freeze.category_of(identity)?;
    Ok(identity)
}

/// ADR-009 B2 (slice S4): register the `trait_ref` / `find_impl` intrinsics
/// against the shared per-compilation-unit freeze handle, mirroring
/// `register_frozen_reflection_builtins` (each closure clones the
/// `Arc<FreezeOverlay>`; the base index is shared, never rebuilt).
///
/// - The trait-ref intrinsic receives the frozen trait identity as two int
///   halves (identity-literal transport from the site rewrite in
///   `comptime.rs` — never a name string) and issues the opaque `TraitRef`
///   carrier.
/// - The find-impl intrinsic consumes a `TypeRef` + `TraitRef` carrier pair
///   and answers ONLY from the frozen impl evidence (freeze input 5): an
///   implemented pair is `Some(ImplRef)`, an unimplemented pair is `None`
///   (R9 — never an error, never partial evidence), and the S2 ruled
///   stances (blanket-impl satisfaction, numeric widening, ambiguous
///   attribution) surface as named stops. The legacy `trait_impl_keys` set
///   is never consulted.
/// `site_time_impl_keys` (slice S5) is the caller's registration-key set as
/// visible AT THE COMPTIME SITE (the live `trait_impl_keys()` snapshot plus
/// the J-CT.2 `comptime impl` pairs threaded into the mini-program). It is
/// consulted for exactly ONE purpose: when frozen evidence answers `None`,
/// a key match proves the pair's impl registered AFTER the freeze barrier,
/// and `find_impl` surfaces the named Dec 52 ordering stop
/// ([`super::semantic_freeze::POST_BARRIER_IMPL_NOT_EVIDENCE_DIAGNOSTIC`])
/// instead of masquerading the pair as unimplemented. It is NEVER an
/// evidence source — no `Some(ImplRef)` is ever produced from it (the S4
/// empty-keys e2e unit test pins that evidence flows only from the freeze).
pub(crate) fn register_trait_evidence_builtins(
    module: &mut ModuleExports,
    freeze: Arc<FreezeOverlay>,
    site_time_impl_keys: std::collections::HashSet<String>,
) {
    let int_param = |name: &str| ModuleParam {
        name: name.to_string(),
        type_name: "int".to_string(),
        required: true,
        ..Default::default()
    };
    let carrier_param = |name: &str, schema: &str| ModuleParam {
        name: name.to_string(),
        type_name: schema.to_string(),
        required: true,
        ..Default::default()
    };

    let freeze_for_trait_ref = Arc::clone(&freeze);
    register_typed_function(
        module,
        super::TRAIT_REF_INTRINSIC,
        "Create an opaque TraitRef from a compiler-issued frozen trait identity",
        vec![int_param("identity_high"), int_param("identity_low")],
        ConcreteType::OpaqueTypedObject(COMPTIME_FROZEN_TRAIT_REF_SCHEMA.to_string()),
        move |slots, _ctx| {
            let identity_part = |index: usize| {
                slots
                    .get(index)
                    .and_then(KindedSlot::as_i64)
                    .ok_or_else(|| "internal trait_ref identity transport is invalid".to_string())
            };
            let identity = FrozenTypeIdentity {
                high: identity_part(0)?,
                low: identity_part(1)?,
            };
            if identity == FrozenTypeIdentity::INVALID {
                // The site rewrite found no frozen trait identity for the
                // bare identifier: not a declared trait (R2 shape).
                return Err(TRAIT_REF_NOT_A_TRAIT_DIAGNOSTIC.to_string());
            }
            let trait_ref = build_frozen_trait_ref_heap_value(identity, &freeze_for_trait_ref)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(trait_ref),
            )))
        },
    );

    let freeze_for_find_impl = freeze;
    register_typed_function(
        module,
        super::FIND_IMPL_INTRINSIC,
        "Look up frozen implementation evidence for an exact (type, trait) identity pair",
        vec![
            carrier_param("type_ref", COMPTIME_FROZEN_TYPE_REF_SCHEMA),
            carrier_param("trait_ref", COMPTIME_FROZEN_TRAIT_REF_SCHEMA),
        ],
        ConcreteType::Option(Box::new(ConcreteType::OpaqueTypedObject(
            COMPTIME_FROZEN_IMPL_REF_SCHEMA.to_string(),
        ))),
        move |slots, _ctx| {
            let type_slot = slots
                .first()
                .ok_or_else(|| "find_impl expects a TypeRef and a TraitRef".to_string())?;
            let trait_slot = slots
                .get(1)
                .ok_or_else(|| "find_impl expects a TypeRef and a TraitRef".to_string())?;
            let type_identity =
                frozen_type_identity_from_type_ref(type_slot, &freeze_for_find_impl)?;
            let trait_identity = frozen_trait_identity_from_ref(trait_slot, &freeze_for_find_impl)?;
            match freeze_for_find_impl.impl_evidence_of(trait_identity, type_identity)? {
                // R9: an unimplemented pair is None — never an error, never
                // a default/partial ImplRef. But a pair whose impl
                // registered AFTER the freeze barrier is a NAMED ordering
                // stop (S1 ruled stance, landed S5), never a silent None.
                None => {
                    if pair_registered_after_barrier(
                        &site_time_impl_keys,
                        &freeze_for_find_impl,
                        trait_identity,
                        type_identity,
                    ) {
                        return Err(
                            super::semantic_freeze::POST_BARRIER_IMPL_NOT_EVIDENCE_DIAGNOSTIC
                                .to_string(),
                        );
                    }
                    Ok(TypedReturn::None)
                }
                Some(set) => {
                    let evidence = set
                        .default_impl()
                        .ok_or_else(|| FIND_IMPL_NAMED_IMPLS_ONLY_DIAGNOSTIC.to_string())?;
                    let impl_ref = build_impl_ref_heap_value(evidence)?;
                    Ok(TypedReturn::Some(ConcreteReturn::OpaqueTypedObject(
                        Arc::new(impl_ref),
                    )))
                }
            }
        },
    );
}

/// S5 (Dec 52 ordering diagnostic ONLY — see the contract note on
/// [`register_trait_evidence_builtins`]): true when the site-time key set
/// records a registration for this exact frozen `(trait, type)` pair even
/// though the freeze issued no evidence for it — i.e. the impl registered
/// after the barrier. Names come from the freeze's own reverse identity
/// maps (registered trait def names, frozen type names incl. synonyms), so
/// an impl registered under a name the freeze never declared can never
/// match here; key forms mirror `environment/registry.rs::trait_impl_keys`
/// (legacy `Trait::Type` + canonical `Trait::Type::ImplName`).
fn pair_registered_after_barrier(
    site_time_impl_keys: &std::collections::HashSet<String>,
    freeze: &FreezeOverlay,
    trait_identity: FrozenTypeIdentity,
    type_identity: FrozenTypeIdentity,
) -> bool {
    if site_time_impl_keys.is_empty() {
        return false;
    }
    for trait_name in freeze.trait_names_for_identity(trait_identity) {
        for type_name in freeze.type_names_for_identity(type_identity) {
            let legacy = format!("{trait_name}::{type_name}");
            let canonical_prefix = format!("{trait_name}::{type_name}::");
            if site_time_impl_keys.contains(&legacy)
                || site_time_impl_keys
                    .iter()
                    .any(|key| key.starts_with(&canonical_prefix))
            {
                return true;
            }
        }
    }
    false
}

/// Resolve a slot to the storage of a RESERVED carrier schema, by name.
/// The reserved names are SOH-prefixed and unspellable in Shape source —
/// this check is the structural R7 barrier described in the module doc.
fn reserved_carrier_storage<'a>(
    slot: &'a KindedSlot,
    expected_schema_name: &str,
) -> Result<(TypeSchema, &'a TypedObjectStorage), String> {
    let carrier_label = carrier_label(expected_schema_name);
    if slot.kind() != NativeKind::Ptr(HeapKind::TypedObject) {
        return Err(format!("expected a compiler-issued {carrier_label} value"));
    }
    let storage = slot
        .as_typed_object_storage()
        .ok_or_else(|| format!("received a null {carrier_label} value"))?;
    let schema = shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
        .ok_or_else(|| {
        format!(
            "could not resolve {carrier_label} schema id {}",
            storage.schema_id
        )
    })?;
    if schema.name != expected_schema_name {
        return Err(format!(
            "expected {carrier_label}, got '{}' — only the compiler issues {carrier_label} values",
            schema.name
        ));
    }
    Ok((schema, storage))
}

/// Human-readable label for the reserved carrier schemas (diagnostics only —
/// identity semantics never depend on it).
fn carrier_label(schema_name: &str) -> &'static str {
    if schema_name == COMPTIME_FROZEN_IMPL_REF_SCHEMA {
        "ImplRef"
    } else if schema_name == COMPTIME_FROZEN_TYPE_REF_SCHEMA {
        "TypeRef"
    } else {
        "TraitRef"
    }
}

/// Read one 128-bit canonical identity out of two integer identity-half
/// fields of a reserved carrier.
fn identity_halves(
    schema: &TypeSchema,
    storage: &TypedObjectStorage,
    high_field: &str,
    low_field: &str,
) -> Result<FrozenTypeIdentity, String> {
    let read = |name: &str| -> Result<i64, String> {
        let field = schema
            .get_field(name)
            .ok_or_else(|| format!("carrier schema '{}' has no {name} field", schema.name))?;
        storage
            .clone_field_kinded(field.index as usize)
            .and_then(|value| value.as_i64())
            .ok_or_else(|| format!("carrier field {name} is not an integer"))
    };
    Ok(FrozenTypeIdentity {
        high: read(high_field)?,
        low: read(low_field)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::BytecodeCompiler;
    use crate::compiler::comptime_builtins::semantic_freeze::SemanticFreeze;
    use crate::compiler::comptime_builtins::type_reflection::build_frozen_type_ref_heap_value;
    use std::sync::Arc;

    fn trait_def(name: &str) -> shape_ast::ast::TraitDef {
        shape_ast::ast::TraitDef {
            name: name.to_string(),
            doc_comment: None,
            type_params: None,
            super_traits: Vec::new(),
            members: Vec::new(),
            annotations: Vec::new(),
            is_comptime: false,
        }
    }

    /// A real freeze (single barrier, no field poking) over: trait
    /// `Greetable`, structs `User` and `Ghost`, a default impl and a named
    /// impl `Loud` for `User` (no impl at all for `Ghost`).
    fn overlay_with_greetable_user() -> Arc<FreezeOverlay> {
        let mut compiler = BytecodeCompiler::new();
        compiler.struct_types.insert(
            "User".to_string(),
            (Vec::new(), shape_ast::ast::Span::DUMMY),
        );
        compiler.struct_types.insert(
            "Ghost".to_string(),
            (Vec::new(), shape_ast::ast::Span::DUMMY),
        );
        compiler
            .type_inference
            .env
            .define_trait(&trait_def("Greetable"));
        compiler
            .type_inference
            .env
            .register_trait_impl("Greetable", "User", vec!["greet".to_string()])
            .expect("default impl registers");
        compiler
            .type_inference
            .env
            .register_trait_impl_named("Greetable", "User", "Loud", vec!["greet".to_string()])
            .expect("named impl registers");
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");
        Arc::new(FreezeOverlay::new(freeze, "<module>", &[]))
    }

    /// Move a built carrier's single refcount share into a `KindedSlot` so
    /// the decode path under test sees the production slot shape.
    fn slot_of(heap: HeapValue) -> KindedSlot {
        let HeapValue::TypedObject(ptr) = heap else {
            panic!("carrier must be a typed object");
        };
        KindedSlot::from_typed_object_raw(ptr.into_raw())
    }

    #[test]
    fn trait_ref_carrier_roundtrips_the_frozen_trait_identity() {
        let overlay = overlay_with_greetable_user();
        let identity = overlay
            .trait_identity_of("Greetable")
            .expect("trait identity frozen");

        let carrier = build_frozen_trait_ref_heap_value(identity, &overlay)
            .expect("freeze-issued identity builds a carrier");
        let slot = slot_of(carrier);
        let decoded = frozen_trait_identity_from_ref(&slot, &overlay)
            .expect("compiler-issued carrier decodes");
        assert_eq!(decoded, identity);
    }

    /// R7 both directions: an identity the freeze never issued is rejected
    /// at carrier CONSTRUCTION, and a hand-built carrier holding arbitrary
    /// identity halves is rejected at DECODE — both with the named
    /// diagnostic, never a usable TraitRef.
    #[test]
    fn trait_ref_carrier_rejects_identities_the_freeze_never_issued() {
        let overlay = overlay_with_greetable_user();
        let unfrozen = FrozenTypeIdentity::from_canonical_descriptor("trait:NeverDeclared");

        let build_rejection = build_frozen_trait_ref_heap_value(unfrozen, &overlay)
            .expect_err("unfrozen identity must not build");
        assert_eq!(build_rejection, FORGED_TRAIT_REF_DIAGNOSTIC);

        // Value-type identities are a DISTINCT kind: a frozen TYPE identity
        // never builds a TraitRef either.
        let type_identity = overlay.identity_of("User").expect("type identity frozen");
        assert_eq!(
            build_frozen_trait_ref_heap_value(type_identity, &overlay)
                .expect_err("a value-type identity must not build a TraitRef"),
            FORGED_TRAIT_REF_DIAGNOSTIC
        );

        let forged = typed_object_for_named_schema(
            COMPTIME_FROZEN_TRAIT_REF_SCHEMA,
            &[
                ("identity_high", KindedSlot::from_int(0x5EED)),
                ("identity_low", KindedSlot::from_int(0xF00D)),
            ],
        );
        let decode_rejection = frozen_trait_identity_from_ref(&forged, &overlay)
            .expect_err("forged identity halves must not decode");
        assert_eq!(decode_rejection, FORGED_TRAIT_REF_DIAGNOSTIC);
    }

    /// The decode is schema-name-checked: a TypeRef carrier (reserved but
    /// WRONG schema) and a plain scalar both fail with named errors — a
    /// TraitRef is a distinct identity kind, never interchangeable with a
    /// TypeRef.
    #[test]
    fn trait_ref_decode_is_schema_name_checked() {
        let overlay = overlay_with_greetable_user();
        let int_identity = overlay.identity_of("int").expect("int identity");
        let type_ref = slot_of(
            build_frozen_type_ref_heap_value(int_identity, &overlay)
                .expect("TypeRef carrier builds"),
        );
        let rejection = frozen_trait_identity_from_ref(&type_ref, &overlay)
            .expect_err("a TypeRef carrier is not a TraitRef");
        assert!(rejection.contains("TraitRef"), "{rejection}");

        let scalar = KindedSlot::from_int(7);
        assert!(frozen_trait_identity_from_ref(&scalar, &overlay).is_err());
    }

    #[test]
    fn impl_ref_carrier_ties_evidence_to_the_exact_pair_and_impl() {
        let overlay = overlay_with_greetable_user();
        let trait_identity = overlay.trait_identity_of("Greetable").expect("trait id");
        let type_identity = overlay.identity_of("User").expect("type id");
        let base = Arc::clone(overlay.base());
        let set = base
            .impl_evidence_of(trait_identity, type_identity)
            .expect("implemented pair is never a surface-and-stop")
            .expect("implemented pair has evidence");

        let default_evidence = set.default_impl().expect("default impl evidence");
        let carrier =
            slot_of(build_impl_ref_heap_value(default_evidence).expect("frozen evidence builds"));
        let decoded = impl_ref_identities_from_evidence_slot(&carrier, &overlay)
            .expect("compiler-issued evidence decodes");
        assert_eq!(
            decoded,
            ImplRefIdentities {
                trait_identity,
                type_identity,
                impl_identity: default_evidence.identity,
            }
        );

        // A named impl is DISTINCT evidence: same pair, different impl
        // identity — both roundtrip, and they never collide.
        let named_evidence = &set.named_impls()[0];
        let named_carrier =
            slot_of(build_impl_ref_heap_value(named_evidence).expect("named evidence builds"));
        let named_decoded = impl_ref_identities_from_evidence_slot(&named_carrier, &overlay)
            .expect("named evidence decodes");
        assert_eq!(named_decoded.trait_identity, trait_identity);
        assert_eq!(named_decoded.type_identity, type_identity);
        assert_eq!(named_decoded.impl_identity, named_evidence.identity);
        assert_ne!(named_decoded.impl_identity, decoded.impl_identity);
    }

    /// R7 for evidence: a hand-built object in the reserved ImplRef schema
    /// with identities the freeze never issued, an ImplRef whose impl half
    /// does not belong to the pair's frozen evidence set, and a TypeRef
    /// carrier — all named rejections, never usable evidence.
    #[test]
    fn impl_ref_decode_rejects_forged_or_lookalike_evidence() {
        let overlay = overlay_with_greetable_user();
        let trait_identity = overlay.trait_identity_of("Greetable").expect("trait id");
        let type_identity = overlay.identity_of("User").expect("type id");

        let forged = typed_object_for_named_schema(
            COMPTIME_FROZEN_IMPL_REF_SCHEMA,
            &[
                ("trait_identity_high", KindedSlot::from_int(1)),
                ("trait_identity_low", KindedSlot::from_int(2)),
                ("type_identity_high", KindedSlot::from_int(3)),
                ("type_identity_low", KindedSlot::from_int(4)),
                ("impl_identity_high", KindedSlot::from_int(5)),
                ("impl_identity_low", KindedSlot::from_int(6)),
            ],
        );
        assert_eq!(
            impl_ref_identities_from_evidence_slot(&forged, &overlay),
            Err(FORGED_IMPL_EVIDENCE_DIAGNOSTIC.to_string())
        );

        // Correct pair, forged impl half: still not freeze-issued evidence.
        let wrong_impl = FrozenTypeIdentity::from_canonical_descriptor("impl:Forged:User:x");
        let half_forged = typed_object_for_named_schema(
            COMPTIME_FROZEN_IMPL_REF_SCHEMA,
            &[
                (
                    "trait_identity_high",
                    KindedSlot::from_int(trait_identity.high),
                ),
                (
                    "trait_identity_low",
                    KindedSlot::from_int(trait_identity.low),
                ),
                (
                    "type_identity_high",
                    KindedSlot::from_int(type_identity.high),
                ),
                ("type_identity_low", KindedSlot::from_int(type_identity.low)),
                ("impl_identity_high", KindedSlot::from_int(wrong_impl.high)),
                ("impl_identity_low", KindedSlot::from_int(wrong_impl.low)),
            ],
        );
        assert_eq!(
            impl_ref_identities_from_evidence_slot(&half_forged, &overlay),
            Err(FORGED_IMPL_EVIDENCE_DIAGNOSTIC.to_string())
        );

        // Schema-name check: a TypeRef carrier is not implementation evidence.
        let int_identity = overlay.identity_of("int").expect("int identity");
        let type_ref = slot_of(
            build_frozen_type_ref_heap_value(int_identity, &overlay)
                .expect("TypeRef carrier builds"),
        );
        let rejection = impl_ref_identities_from_evidence_slot(&type_ref, &overlay)
            .expect_err("a TypeRef carrier is not an ImplRef");
        assert!(rejection.contains("ImplRef"), "{rejection}");
    }

    /// S4 DoD-critical (Dec 49: "the compiler hashes canonical trait and
    /// implementation identities into generated artifacts"): the evidence
    /// artifact a comptime `find_impl → Some(proof)` execution produces
    /// carries EXACTLY the canonical SHA-256 descriptor identities the
    /// freeze issued — `trait:Greetable` and
    /// `impl:Greetable:User:__default__` — end-to-end through the real
    /// comptime mini-VM (rewrite → forwarder → intrinsic → Option marshal →
    /// Some-arm binding), not through any name-based lookup.
    ///
    /// `trait_impl_keys` is passed EMPTY: the legacy per-site key set the
    /// deleted A1 pattern re-read cannot be the evidence source — frozen
    /// barrier truth is.
    #[test]
    fn find_impl_evidence_artifact_carries_canonical_freeze_identities() {
        let program = shape_ast::parse_program(
            r#"
fn __t__() {
  return match find_impl(type_ref(User), trait_ref(Greetable)) {
    Some(proof) => proof
    None => error("unreachable: the pair is implemented")
  }
}
"#,
        )
        .expect("evidence-consuming body parses");
        let body = program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(def, _) => Some(def.body.clone()),
                _ => None,
            })
            .expect("wrapper function present");

        let overlay = overlay_with_greetable_user();
        let execution = crate::compiler::comptime::execute_comptime(
            &body,
            &[],
            &[],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            Arc::clone(&overlay),
        )
        .expect("evidence-consuming comptime execution succeeds");

        let identities = impl_ref_identities_from_evidence_slot(&execution.value, &overlay)
            .expect("the produced artifact decodes as compiler-issued evidence");
        assert_eq!(
            identities.trait_identity,
            FrozenTypeIdentity::from_canonical_descriptor("trait:Greetable"),
            "the artifact must carry the canonical trait descriptor hash"
        );
        assert_eq!(
            Some(identities.type_identity),
            overlay.identity_of("User"),
            "the artifact must carry the frozen type identity of the exact pair"
        );
        assert_eq!(
            identities.impl_identity,
            FrozenTypeIdentity::from_canonical_descriptor("impl:Greetable:User:__default__"),
            "the artifact must carry the canonical impl descriptor hash"
        );
    }

    /// R1 (slice S5): a frozen TRAIT identity never builds a `TypeRef` —
    /// the rejection is the NAMED traits-are-not-value-types diagnostic at
    /// the carrier builder, and the same named text surfaces end-to-end
    /// through the real comptime execution of `type_ref(TraitName)` (the
    /// site rewrite falls through to the frozen trait identity so the
    /// intrinsic can name the rejection instead of answering A1's generic
    /// unknown-identity text).
    #[test]
    fn type_ref_carrier_rejects_a_frozen_trait_identity_with_the_named_r1_text() {
        let overlay = overlay_with_greetable_user();
        let trait_identity = overlay
            .trait_identity_of("Greetable")
            .expect("trait identity frozen");

        let build_rejection = build_frozen_type_ref_heap_value(trait_identity, &overlay)
            .expect_err("a trait identity must not build a TypeRef");
        assert_eq!(build_rejection, TRAIT_NOT_A_VALUE_TYPE_DIAGNOSTIC);

        // End-to-end through the mini-VM: `type_ref(Greetable)` is the same
        // named rejection...
        let execution = execute_comptime_source("fn __t__() { return type_ref(Greetable) }");
        let error = expect_comptime_error(execution, "type_ref over a trait must fail");
        assert!(
            error.to_string().contains("traits are not value types"),
            "{error}"
        );

        // ...while a genuinely-unknown name keeps A1 row 2's generic
        // diagnostic (no regression from the R1 upgrade).
        let unknown = execute_comptime_source("fn __t__() { return type_ref(DoesNotExist) }");
        let error = expect_comptime_error(unknown, "unknown name must still fail");
        assert!(
            error.to_string().contains("unknown semantic type identity"),
            "{error}"
        );
    }

    /// Unwrap the error arm of a comptime execution (the Ok carrier has no
    /// `Debug`, so `Result::expect_err` is unavailable).
    fn expect_comptime_error(
        result: shape_ast::error::Result<crate::compiler::comptime::ComptimeExecutionResult>,
        what: &str,
    ) -> shape_ast::error::ShapeError {
        match result {
            Ok(_) => panic!("{what}: expected an error, got a value"),
            Err(error) => error,
        }
    }

    /// S1-ruled stance, S5 named diagnostic: a pair whose implementation was
    /// registered AFTER the semantic-freeze barrier (visible in the
    /// site-time key set but absent from frozen evidence) is a named
    /// ordering stop — never a silent `find_impl -> None`. The empty-keys
    /// control (`find_impl_unimplemented_pair_executes_the_none_arm` below)
    /// pins that a genuinely-unimplemented pair still answers `None`.
    #[test]
    fn post_barrier_registered_pair_is_the_named_ordering_stop_never_a_silent_none() {
        let program = shape_ast::parse_program(
            r#"
fn __t__() {
  return match find_impl(type_ref(Ghost), trait_ref(Greetable)) {
    Some(proof) => 1
    None => 0
  }
}
"#,
        )
        .expect("post-barrier body parses");
        let body = program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(def, _) => Some(def.body.clone()),
                _ => None,
            })
            .expect("wrapper function present");

        let overlay = overlay_with_greetable_user();
        // The site-time key set says Greetable::Ghost is registered; the
        // freeze (barrier truth) has no such evidence — i.e. the impl
        // registered after the barrier (comptime-generated / derived).
        let mut site_time_keys = std::collections::HashSet::new();
        site_time_keys.insert("Greetable::Ghost".to_string());
        let execution = crate::compiler::comptime::execute_comptime(
            &body,
            &[],
            &[],
            site_time_keys,
            std::collections::HashSet::new(),
            Arc::clone(&overlay),
        );
        let error = expect_comptime_error(
            execution,
            "post-barrier pair must be a named stop, not None",
        );
        assert!(
            error
                .to_string()
                .contains("registered after the semantic-freeze barrier"),
            "{error}"
        );
    }

    /// Shared driver: run one wrapper-fn body through the real comptime
    /// mini-VM against the Greetable/User/Ghost freeze with EMPTY site-time
    /// keys.
    fn execute_comptime_source(
        source: &str,
    ) -> shape_ast::error::Result<crate::compiler::comptime::ComptimeExecutionResult> {
        let program = shape_ast::parse_program(source).expect("test body parses");
        let body = program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(def, _) => Some(def.body.clone()),
                _ => None,
            })
            .expect("wrapper function present");
        crate::compiler::comptime::execute_comptime(
            &body,
            &[],
            &[],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            overlay_with_greetable_user(),
        )
    }

    /// S5 named-rejection rows through the REAL mini-VM pipeline (the outer
    /// analyzer deliberately tolerates per-statement inference errors inside
    /// `comptime { }` walks, so the mini-VM analyzer is the enforcement
    /// point these rows pin — routed there by the `find_impl` forwarder's
    /// concrete carrier-schema param annotations, `comptime.rs`):
    /// R8 arity, R3 name-string lookup, R4 boolean-authorized generation.
    #[test]
    fn find_impl_checker_rejections_are_named_in_the_mini_vm() {
        for (src, needle) in [
            (
                "fn __t__() { return match find_impl() { Some(p) => 1 None => 0 } }",
                "find_impl expects exactly two arguments",
            ),
            (
                r#"fn __t__() { return match find_impl(type_ref(User), "Greetable") { Some(p) => 1 None => 0 } }"#,
                "trait lookup cannot use text",
            ),
            (
                r#"fn __t__() { return match find_impl(type_ref(User), true) { Some(p) => 1 None => 0 } }"#,
                "a boolean cannot authorize an operation that requires implementation evidence",
            ),
        ] {
            let error = expect_comptime_error(
                execute_comptime_source(src),
                "forbidden form must be a named rejection",
            );
            assert!(error.to_string().contains(needle), "{src} -> {error}");
        }
    }

    /// R9 at the intrinsic tier: an unimplemented pair flows out of the real
    /// comptime execution as the `None` arm — never an error, never a
    /// default/partial `ImplRef`.
    #[test]
    fn find_impl_unimplemented_pair_executes_the_none_arm() {
        let program = shape_ast::parse_program(
            r#"
fn __t__() {
  return match find_impl(type_ref(int), trait_ref(Greetable)) {
    Some(proof) => 1
    None => 0
  }
}
"#,
        )
        .expect("none-arm body parses");
        let body = program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(def, _) => Some(def.body.clone()),
                _ => None,
            })
            .expect("wrapper function present");

        let overlay = overlay_with_greetable_user();
        let execution = crate::compiler::comptime::execute_comptime(
            &body,
            &[],
            &[],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            Arc::clone(&overlay),
        )
        .expect("unimplemented pair must be None, never an execution error");
        assert_eq!(
            execution.value.as_i64(),
            Some(0),
            "unimplemented pair must select the None arm"
        );
    }
}
