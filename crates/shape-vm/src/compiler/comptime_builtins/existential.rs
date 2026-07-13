//! ADR-009 B3 (Dec 51) — existential descriptor packages. // ADR-009
//!
//! `exists<W...> Descriptor<W...>` is the type-level package that binds a set
//! of hidden witnesses; `comptime for some<W...> x in coll { … }` opens one
//! package per iteration, introducing fresh hidden witness identities. This
//! module is the freeze-model half (slice S2): it canonicalizes the package
//! through the ONE canonicalizer (`type_reflection::canonicalize_type_annotation`,
//! Existential arm) and the ONE freeze query surface (`FreezeOverlay`), and it
//! carries the freeze-boundary rejection diagnostics. The real per-witness
//! unroll (inference / MIR lowering / VM+JIT iteration) lands in slice S3 — it
//! is sugar over a rank-2 generic callback, NOT a second reflection protocol.
//!
//! ## Two witness scopings — one concept, two well-founded roles
//!
//! * **Package identity** (this module's canonicalization): witnesses are
//!   POSITIONAL (`witness:{index}` descriptors, de-Bruijn style) so the
//!   package identity is alpha-invariant (`exists<A,B> Pair<A,B>` and
//!   `exists<I,F> Pair<I,F>` are the same type) and site-independent.
//! * **Opening at a `some` site** (`FreezeOverlay::open_witnesses`): witnesses
//!   are FRESH per site (`parameter:{some_site}:{witness}` descriptors,
//!   modeled on the specialization type-param overlay) so two iterations never
//!   share a witness identity and a witness never escapes its opening scope.
//!
//! These are the standard type-theory distinction between a bound
//! existential's canonical form and its opened (skolemized) instance — not a
//! parallel implementation of one thing (considered-and-kept note logged in
//! `docs/defections.md`).

use super::semantic_freeze::FreezeOverlay;
use super::type_reflection::{FrozenTypeCategory, FrozenTypeIdentity};
use shape_ast::ast::TypeAnnotation;

/// Rejection-matrix row 1 (ADR-009 B3, Dec 51): a heterogeneous witness slot
/// spelled with the compiler-internal top type `Any` erases the witness.
/// DISTINCT from the ENABLED `any` / `dyn Trait` Erased category (ADR-009
/// L60): lowercase `any` is a frozen Erased leaf; the compiler-internal `Any`
/// as a witness slot is this named rejection — a hidden witness must be bound
/// existentially, never erased to the top type.
pub(crate) const WITNESS_ERASED_TO_ANY_DIAGNOSTIC: &str = "heterogeneous witnesses cannot be erased to Any: a hidden witness of an \
     existential descriptor package must be bound existentially, not filled \
     with the compiler-internal Any top type";

/// Rejection-matrix row 6 (ADR-009 B3, Dec 51): `comptime for some<W...>`
/// iterates an existential descriptor collection. An element type that is not
/// an existential descriptor package is this named rejection — never a silent
/// non-iteration. Canonical text is single-sourced in shape-runtime so the
/// inference tier and the freeze tier speak one string (re-export, not copy).
pub(crate) use shape_runtime::comptime_reflection::NON_EXISTENTIAL_ITERABLE_DIAGNOSTIC;

/// Rejection-matrix rows 2 & 3 (ADR-009 B3, Dec 51): witness escape + second
/// reflection protocol. Single-sourced in shape-runtime (see above).
pub(crate) use shape_runtime::comptime_reflection::{
    SECOND_REFLECTION_PROTOCOL_DIAGNOSTIC, WITNESS_ESCAPES_SCOPE_DIAGNOSTIC,
};

/// True when the annotation structurally mentions the compiler-internal `Any`
/// top type (rejection-matrix row 1). Exhaustive over `TypeAnnotation` so a new
/// variant forces this walk to be revisited. Scoped to the existential
/// canonicalization arm: only capital `Any` in a witness-bearing package is the
/// erasure marker (lowercase `any` is the enabled Erased leaf, untouched here).
pub(crate) fn annotation_erases_witness_to_any(annotation: &TypeAnnotation) -> bool {
    fn is_any(name: &str) -> bool {
        name == "Any"
    }
    match annotation {
        TypeAnnotation::Basic(name) => is_any(name),
        TypeAnnotation::Reference(path) => is_any(path.as_str()),
        TypeAnnotation::Array(inner) | TypeAnnotation::Borrow { inner, .. } => {
            annotation_erases_witness_to_any(inner)
        }
        TypeAnnotation::Tuple(items)
        | TypeAnnotation::Union(items)
        | TypeAnnotation::Intersection(items) => {
            items.iter().any(annotation_erases_witness_to_any)
        }
        TypeAnnotation::Object(fields) => fields
            .iter()
            .any(|field| annotation_erases_witness_to_any(&field.type_annotation)),
        TypeAnnotation::Function { params, returns } => {
            params
                .iter()
                .any(|param| annotation_erases_witness_to_any(&param.type_annotation))
                || annotation_erases_witness_to_any(returns)
        }
        TypeAnnotation::Generic { name, args } => {
            is_any(name.as_str()) || args.iter().any(annotation_erases_witness_to_any)
        }
        TypeAnnotation::Existential { inner, .. } => annotation_erases_witness_to_any(inner),
        TypeAnnotation::Dyn(_)
        | TypeAnnotation::Void
        | TypeAnnotation::Never
        | TypeAnnotation::Null
        | TypeAnnotation::Undefined => false,
    }
}

/// The freeze-model gate a `comptime for some<W...>` site runs before opening
/// witnesses (rejection-matrix row 6): canonicalize the iterable's ELEMENT
/// type through the ONE freeze query surface and require it to be an
/// existential descriptor package. Returns the package's frozen identity
/// (interned in the overlay's composite memo) on success.
///
/// Row 5 (no freeze handle) is upstream of this call: the site obtains the
/// `FreezeOverlay` via `BytecodeCompiler::comptime_freeze_overlay`, which is
/// the `NO_FREEZE_HANDLE_DIAGNOSTIC` gate — a site that cannot obtain the
/// per-compilation-unit freeze never reaches this function.
pub(crate) fn require_existential_element(
    overlay: &FreezeOverlay,
    element_type: &TypeAnnotation,
) -> Result<FrozenTypeIdentity, String> {
    let identity = overlay.canonicalize_type(element_type)?;
    match overlay.category_of(identity)? {
        FrozenTypeCategory::Existential => Ok(identity),
        other => Err(format!(
            "{NON_EXISTENTIAL_ITERABLE_DIAGNOSTIC} (found category {})",
            other.variant_name()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::BytecodeCompiler;
    use crate::compiler::comptime_builtins::semantic_freeze::{
        FreezeOverlay, NO_FREEZE_HANDLE_DIAGNOSTIC, SemanticFreeze,
    };
    use shape_ast::ast::{Span, TypeParam, TypePath};

    fn add_generic_struct(compiler: &mut BytecodeCompiler, name: &str, params: &[&str]) {
        compiler
            .struct_types
            .insert(name.to_string(), (Vec::new(), Span::DUMMY));
        compiler.struct_generic_info.insert(
            name.to_string(),
            crate::compiler::StructGenericInfo {
                type_params: params
                    .iter()
                    .map(|param| TypeParam::Type {
                        name: (*param).to_string(),
                        span: Span::DUMMY,
                        doc_comment: None,
                        default_type: None,
                        trait_bounds: Vec::new(),
                    })
                    .collect(),
                runtime_field_types: std::collections::HashMap::new(),
            },
        );
    }

    fn add_struct(compiler: &mut BytecodeCompiler, name: &str) {
        compiler
            .struct_types
            .insert(name.to_string(), (Vec::new(), Span::DUMMY));
    }

    fn overlay_with(configure: impl FnOnce(&mut BytecodeCompiler)) -> FreezeOverlay {
        let mut compiler = BytecodeCompiler::new();
        configure(&mut compiler);
        let freeze = SemanticFreeze::freeze(&compiler).expect("test compiler state must freeze");
        FreezeOverlay::new(freeze, "<module>", &[])
    }

    fn basic(name: &str) -> TypeAnnotation {
        TypeAnnotation::Basic(name.to_string())
    }

    fn applied(head: &str, args: Vec<TypeAnnotation>) -> TypeAnnotation {
        TypeAnnotation::Generic {
            name: TypePath::simple(head),
            args,
        }
    }

    fn existential(witnesses: &[&str], inner: TypeAnnotation) -> TypeAnnotation {
        TypeAnnotation::Existential {
            witnesses: witnesses.iter().map(|w| w.to_string()).collect(),
            inner: Box::new(inner),
        }
    }

    /// RED (a): an existential descriptor package canonicalizes to a stable
    /// FrozenTypeIdentity in the Existential category — reproducible across
    /// repeated canonicalization, alpha-invariant (witness names don't
    /// matter), and DISTINCT per witness arity.
    #[test]
    fn existential_package_canonicalizes_to_a_stable_arity_distinct_identity() {
        let overlay = overlay_with(|compiler| {
            add_struct(compiler, "Owner");
            add_generic_struct(compiler, "Pair", &["A", "B"]);
            add_generic_struct(compiler, "Cell", &["A"]);
        });

        // exists<I,F> Pair<I,F> — arity 2.
        let arity2 = existential(&["I", "F"], applied("Pair", vec![basic("I"), basic("F")]));
        let first = overlay
            .canonicalize_type(&arity2)
            .expect("existential must canonicalize");
        let again = overlay
            .canonicalize_type(&arity2)
            .expect("existential must canonicalize");
        assert_eq!(first, again, "package identity must be stable");
        assert_eq!(
            overlay.category_of(first),
            Ok(FrozenTypeCategory::Existential),
            "existential package classifies as Existential"
        );

        // Alpha-invariance: witness NAMES are irrelevant to identity.
        let renamed = existential(&["X", "Y"], applied("Pair", vec![basic("X"), basic("Y")]));
        assert_eq!(
            overlay.canonicalize_type(&renamed).expect("canonicalizes"),
            first,
            "alpha-equivalent existentials share one identity"
        );

        // Distinct per witness arity: exists<I> Cell<I> is a different type.
        let arity1 = existential(&["I"], applied("Cell", vec![basic("I")]));
        let arity1_id = overlay
            .canonicalize_type(&arity1)
            .expect("existential must canonicalize");
        assert_ne!(
            first, arity1_id,
            "witness arity is descriptor-significant"
        );
    }

    /// RED (b): a witness slot spelled compiler-internal `Any` erases the
    /// witness — rejected with the named WITNESS_ERASED_TO_ANY diagnostic,
    /// distinct from the enabled lowercase-`any` Erased leaf.
    #[test]
    fn witness_slot_erased_to_any_is_rejected() {
        let overlay = overlay_with(|compiler| {
            add_struct(compiler, "Owner");
            add_generic_struct(compiler, "Pair", &["A", "B"]);
        });

        // exists<I,F> Pair<I, Any> — F declared but the slot is erased to Any.
        let erased = existential(&["I", "F"], applied("Pair", vec![basic("I"), basic("Any")]));
        let error = overlay
            .canonicalize_type(&erased)
            .expect_err("erased witness must reject");
        assert_eq!(error, WITNESS_ERASED_TO_ANY_DIAGNOSTIC);

        // Lowercase `any` (the enabled Erased leaf) is NOT the erasure marker.
        let lowercase = existential(&["I", "F"], applied("Pair", vec![basic("I"), basic("any")]));
        overlay
            .canonicalize_type(&lowercase)
            .expect("lowercase any is the enabled Erased leaf, not erasure");
    }

    /// RED (c): a `some` site that cannot obtain the per-compilation-unit
    /// freeze handle fires NO_FREEZE_HANDLE_DIAGNOSTIC — before any iteration
    /// body executes. The handle acquisition (`comptime_freeze_overlay`) is
    /// the single gate every some-site passes through first.
    #[test]
    fn some_site_without_installed_freeze_fires_no_freeze_handle() {
        let compiler = BytecodeCompiler::new();
        assert!(
            compiler.semantic_freeze.is_none(),
            "fresh compiler has no installed freeze"
        );
        let error = compiler
            .comptime_freeze_overlay()
            .expect_err("no-freeze some-site must reject");
        assert!(
            error.to_string().contains(NO_FREEZE_HANDLE_DIAGNOSTIC),
            "no-freeze-handle diagnostic missing: {error}"
        );
    }

    /// RED (d): `comptime for some` over a non-existential element type is the
    /// named NON_EXISTENTIAL_ITERABLE rejection; an existential element type
    /// passes the gate.
    #[test]
    fn non_existential_iterable_is_rejected_and_existential_passes() {
        let overlay = overlay_with(|compiler| {
            add_struct(compiler, "Owner");
            add_generic_struct(compiler, "Pair", &["A", "B"]);
        });

        // A plain applied nominal element type is not an existential package.
        let error = require_existential_element(&overlay, &applied("Array", vec![basic("int")]))
            .expect_err("non-existential element must reject");
        assert!(
            error.contains(NON_EXISTENTIAL_ITERABLE_DIAGNOSTIC),
            "non-existential-iterable diagnostic missing: {error}"
        );

        // A bare primitive is not an existential package either.
        assert!(
            require_existential_element(&overlay, &basic("int"))
                .expect_err("primitive element must reject")
                .contains(NON_EXISTENTIAL_ITERABLE_DIAGNOSTIC)
        );

        // The existential descriptor package passes the gate.
        let element = existential(&["I", "F"], applied("Pair", vec![basic("I"), basic("F")]));
        let identity =
            require_existential_element(&overlay, &element).expect("existential element passes");
        assert_eq!(
            overlay.category_of(identity),
            Ok(FrozenTypeCategory::Existential)
        );
    }

    /// Finding-1 regression (fix round 1): the production `some`-site gate
    /// (`check_comptime_for_some_rejections`, `expressions/misc.rs`) routes
    /// Row 6 through THIS freeze surface — `require_existential_element` ->
    /// `FreezeOverlay::canonicalize_type` -> the canonicalizer's Existential
    /// arm — which canonicalizes the WHOLE package, recursing into the inner
    /// descriptor. It is NOT the shallow, syntactic
    /// `matches!(TypeAnnotation::Existential { .. })` outer check the gate used
    /// before (which left this surface dead relative to production). Proof of
    /// the distinction: an existential whose INNER references a type that is
    /// not frozen in the unit is rejected by the freeze canonicalizer — a
    /// rejection a purely-syntactic outer shape test cannot produce.
    #[test]
    fn require_existential_element_canonicalizes_the_inner_through_the_freeze() {
        let overlay = overlay_with(|compiler| {
            add_struct(compiler, "Owner");
            // `Ghost` is deliberately NOT registered — the inner descriptor
            // references an unfrozen nominal.
        });

        // Syntactically this IS an existential package, so the old outer
        // `matches!(Existential)` check would have ACCEPTED it. The freeze
        // canonicalizer recurses into `Ghost<T>` and rejects the unfrozen head.
        let element = existential(&["T"], applied("Ghost", vec![basic("T")]));
        let error = require_existential_element(&overlay, &element)
            .expect_err("an existential over an unfrozen inner must reject");
        assert!(
            error.contains("not frozen"),
            "expected a freeze-canonicalizer rejection (recursion into the inner \
             descriptor), got: {error}"
        );

        // Control: the same package over a FROZEN inner canonicalizes and
        // classifies as Existential — the gate accepts it.
        let overlay = overlay_with(|compiler| {
            add_struct(compiler, "Owner");
            add_generic_struct(compiler, "Cell", &["A"]);
        });
        let ok = existential(&["T"], applied("Cell", vec![basic("T")]));
        let identity = require_existential_element(&overlay, &ok)
            .expect("existential over a frozen inner passes the gate");
        assert_eq!(
            overlay.category_of(identity),
            Ok(FrozenTypeCategory::Existential)
        );
    }

    /// GREEN companion: opening witnesses at a `some` site mints FRESH
    /// per-site `parameter:{some_site}:{witness}` identities (modeled on the
    /// specialization type-param overlay). Two distinct sites never share a
    /// witness identity; the same site is reproducible.
    #[test]
    fn open_witnesses_scopes_fresh_per_site_identities() {
        let overlay = overlay_with(|_| {});
        let witnesses = ["I".to_string(), "F".to_string()];

        let site_a = overlay.open_witnesses("some@a", &witnesses);
        let site_b = overlay.open_witnesses("some@b", &witnesses);

        for name in ["I", "F"] {
            assert!(site_a.is_scoped_parameter(name), "{name} scoped at site a");
            assert!(site_b.is_scoped_parameter(name), "{name} scoped at site b");
            assert_eq!(
                site_a.category_of(site_a.identity_of(name).expect("witness identity")),
                Ok(FrozenTypeCategory::Parameter),
                "opened witness classifies as Parameter"
            );
            assert_ne!(
                site_a.identity_of(name),
                site_b.identity_of(name),
                "{name} must be a fresh identity per opening site"
            );
        }

        // Reproducible: re-opening the same site yields the same identities.
        let site_a_again = overlay.open_witnesses("some@a", &witnesses);
        assert_eq!(site_a.identity_of("I"), site_a_again.identity_of("I"));
    }

    /// Rejection-matrix row 3 (ADR-009 B3, Dec 51): `comptime for some` is
    /// iteration sugar over the SINGLE reflect()/payload freeze surface — it
    /// desugars to the existing `comptime_mode` runtime-for-loop rewrite, never
    /// a parallel iterator or a second reflection protocol. The enforcement is
    /// architectural (surface-and-stop: there is no user syntax that requests a
    /// second protocol), and the named diagnostic exists so any future attempt
    /// to add one has a refusal to point at. This pins the const's presence and
    /// its single-sourced text.
    #[test]
    fn second_reflection_protocol_is_a_named_surface_and_stop_refusal() {
        assert_eq!(
            SECOND_REFLECTION_PROTOCOL_DIAGNOSTIC,
            shape_runtime::comptime_reflection::SECOND_REFLECTION_PROTOCOL_DIAGNOSTIC,
            "the row-3 diagnostic must be single-sourced in shape-runtime"
        );
        assert!(SECOND_REFLECTION_PROTOCOL_DIAGNOSTIC.contains("second reflection"));
    }

    /// A witness always shadows: a base type of the same spelling never
    /// captures a freshly-opened hidden witness.
    #[test]
    fn opened_witness_shadows_a_base_type_of_the_same_name() {
        let overlay = overlay_with(|compiler| add_struct(compiler, "Owner"));
        let opened = overlay.open_witnesses("some@x", &["Owner".to_string()]);
        assert!(opened.is_scoped_parameter("Owner"));
        assert_eq!(
            opened.category_of(opened.identity_of("Owner").expect("witness identity")),
            Ok(FrozenTypeCategory::Parameter),
            "opened witness shadows the base nominal of the same name"
        );
        // The base freeze still classifies Owner as its nominal identity.
        assert_eq!(
            overlay.category_of(overlay.identity_of("Owner").expect("Owner nominal")),
            Ok(FrozenTypeCategory::Nominal)
        );
    }
}
