//! ADR-009 C3 #14 (slice 1) — the typed template carrier for annotation
//! runtime hooks: `CheckedTemplate` + `CheckedTemplateBuilder<SigState,
//! CapturesState>`.
//!
//! # Discharging the checked_body.rs:52-56 deferral (the semantic Sig/Captures face)
//!
//! The sibling `checked_body` module deferred, verbatim: "The typestate
//! parameters `SigState`/`CapturesState` track SUPPLIED-NESS only; they are
//! named distinctly from Decision-95's SEMANTIC `CheckedBody<Sig, Captures>`
//! type parameters (the Shape comptime-type face, which lands with the
//! Decision-95 Shape staging surface in a later E-track/C3 slice)." THIS module
//! is that slice, and the resolved spelling is: a Shape-level signature cannot
//! be a Rust type parameter (it is DATA), so the semantic `Sig`/`Captures` face
//! lands as CARRIED SEMANTIC DATA on a non-generic [`CheckedTemplate`] — a
//! [`TemplateSig`] (the Sig face: polymorphic-args, polymorphic-result, or a
//! concrete `BodySignature`) plus the shipped C1 `CaptureClause` (the Captures
//! face). The Shape-face type parameters `CheckedTemplate<Sig, Captures>`
//! become S2's comptime-type PROJECTION of this data — the public comptime API
//! surfaces them as Shape types; no Rust generic ever carries them. The
//! builder's `SigState`/`CapturesState` typestate parameters here remain, as in
//! `checked_body.rs:52-56`, SUPPLIED-NESS trackers only.
//!
//! # Construction chokepoint (the same split as `checked_body`)
//!
//! This is the CONSTRUCTION side: `finish()` validates the typed inputs and
//! returns a [`CheckedTemplate`] or a named `ShapeError` — never a silent
//! partial. It does not specialize, install, stamp, or publish; the
//! specialization/install side is `crate::compiler::template_specialization`
//! (S1c), which composes with the ALREADY-open C2 `InstallTransaction` exactly
//! as `checked_body`'s consumers do. `finish()` exists ONLY on
//! `<Present, Present>` — finishing a builder whose body fn or capture set was
//! never supplied is unrepresentable (the `ProofGap` discipline).
//!
//! # No source-string escape hatch (C3-G3: code is code)
//!
//! There is deliberately NO `from_source(&str)` / string / JSON constructor
//! anywhere on this surface. A template's body is an ORDINARY TYPED SHAPE
//! FUNCTION ([`CheckedTemplateBuilder::body_fn`] takes the parsed
//! `&FunctionDef`); the only way to obtain a [`CheckedTemplate`] is through the
//! typestate builder.
//!
//! # Non-serializable, compiler-session-local
//!
//! Templates are COMPILER-SESSION-LOCAL: no type in this module derives
//! serde, and none may grow it — a `CheckedTemplate` names a body fn in the
//! CURRENT compilation's registry, so a serialized template would smuggle a
//! cross-session name reference with no provenance behind it (the same class
//! of hole the C1 `GeneratedNodeOrigin` issuer check closes for closures).
//! Carrying templates across a serialization boundary is a named follow-up
//! (see the carried-forward carrier design in
//! `docs/design/typed-comptime/c3-decisions.md`), not a supported path.

use std::marker::PhantomData;

use shape_ast::ast::{CaptureClause, FunctionDef, Statement, TypeAnnotation, TypeParam};
use shape_ast::error::{Result, ShapeError};

use super::checked_body::{validate_capture_clause, BodySignature, Missing, Present};
use crate::compiler::template_specialization::pseudo_tuple::validate_pseudo_tuple_uses;

/// Which runtime hook a template body is written for. Drives classification:
/// the one-type-param polymorphic form reads as the args pseudo-tuple for a
/// `before` hook and as the typed result for an `after` hook (C3-G4/G9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum TemplateHookKind {
    /// Runs before the target; a polymorphic body takes and returns the typed
    /// args pack (`fn t<Args>(args: Args) -> Args`).
    Before,
    /// Runs after the target; a polymorphic body takes and returns the typed
    /// result (`fn t<R>(result: R) -> R`).
    After,
}

/// The Sig face of a checked template — the SEMANTIC signature data the
/// Decision-95 `CheckedTemplate<Sig, Captures>` type parameters project
/// (see the module docs).
#[derive(Debug, Clone)]
pub(in crate::compiler) enum TemplateSig {
    /// `fn t<Args>(args: Args) -> Args` — Before only (the C3-G9 pseudo-tuple
    /// binding: `args[i]` / `args.length` resolve at specialization; no tuple
    /// value ever exists).
    PolymorphicArgs {
        type_param: String,
        args_param: String,
    },
    /// `fn t<R>(result: R) -> R` — After only (the typed result flows through).
    PolymorphicResult {
        type_param: String,
        result_param: String,
    },
    /// A concrete signature — the C3-G4 degenerate case: checked at
    /// definition under its own signature; match-or-error against the frozen
    /// target at the `@application` site (S1c).
    Concrete(BodySignature),
}

/// A checked annotation-hook template: one hook kind, one body fn (named,
/// compiler-session-local), its classified [`TemplateSig`], and its complete
/// explicitly-declared capture set. Built ONLY through
/// [`CheckedTemplateBuilder::finish`]; the private fields and absent public
/// constructor mean no compiler module can assemble one around the
/// classification rule (so a `PolymorphicArgs` sig can never be paired with an
/// `After` hook, and vice versa — the classifier derives the variant FROM the
/// hook kind).
#[derive(Debug)]
pub(in crate::compiler) struct CheckedTemplate {
    hook_kind: TemplateHookKind,
    body_fn: String,
    sig: TemplateSig,
    captures: CaptureClause,
    arity: usize,
    type_param_count: usize,
}

impl CheckedTemplate {
    /// Which hook this template implements.
    pub(in crate::compiler) fn hook_kind(&self) -> TemplateHookKind {
        self.hook_kind
    }

    /// The template body fn's name in the current compilation.
    pub(in crate::compiler) fn body_fn(&self) -> &str {
        &self.body_fn
    }

    /// The classified Sig face.
    pub(in crate::compiler) fn sig(&self) -> &TemplateSig {
        &self.sig
    }

    /// The complete, explicitly-declared capture set (the Captures face).
    pub(in crate::compiler) fn captures(&self) -> &CaptureClause {
        &self.captures
    }

    /// The body fn's value-parameter count (1 for the polymorphic forms).
    pub(in crate::compiler) fn arity(&self) -> usize {
        self.arity
    }

    /// The body fn's type-parameter count (1 for the polymorphic forms, 0 for
    /// concrete).
    pub(in crate::compiler) fn type_param_count(&self) -> usize {
        self.type_param_count
    }
}

/// The sig-side snapshot taken by [`CheckedTemplateBuilder::body_fn`]. The
/// body statements are retained ONLY for `finish()`'s pseudo-tuple validation
/// (they are not carried on [`CheckedTemplate`] — the template references its
/// body fn by name; the fn itself stays in the ordinary registry).
struct BodyFnSnapshot {
    name: String,
    sig: TemplateSig,
    arity: usize,
    type_param_count: usize,
    body: Vec<Statement>,
}

/// The typed builder for a [`CheckedTemplate`] — EXACTLY the
/// `CheckedBodyBuilder` typestate pattern (`checked_body.rs`): start with
/// [`Self::new`], supply the body fn ([`CheckedTemplateBuilder::body_fn`]) and
/// the capture set ([`CheckedTemplateBuilder::captures`]) in any order, then
/// [`finish`] — which exists ONLY once BOTH axes are [`Present`].
///
/// [`finish`]: CheckedTemplateBuilder::finish
pub(in crate::compiler) struct CheckedTemplateBuilder<SigState, CapturesState> {
    hook_kind: TemplateHookKind,
    body_fn: Option<BodyFnSnapshot>,
    captures: Option<CaptureClause>,
    _sig: PhantomData<SigState>,
    _captures: PhantomData<CapturesState>,
}

impl CheckedTemplateBuilder<Missing, Missing> {
    /// A fresh builder for the given hook kind — no body fn, no capture set.
    pub(in crate::compiler) fn new(hook_kind: TemplateHookKind) -> Self {
        Self {
            hook_kind,
            body_fn: None,
            captures: None,
            _sig: PhantomData,
            _captures: PhantomData,
        }
    }
}

impl<SigState, CapturesState> CheckedTemplateBuilder<SigState, CapturesState> {
    /// Supply the template body fn (an ordinary typed Shape function, C3-G3),
    /// advancing `SigState` to [`Present`]. Validates and CLASSIFIES the
    /// signature (see [`classify_template_sig`]) and snapshots the fn's name,
    /// arity, type-parameter count, and body statements.
    pub(in crate::compiler) fn body_fn(
        self,
        func: &FunctionDef,
    ) -> Result<CheckedTemplateBuilder<Present, CapturesState>> {
        let sig = classify_template_sig(self.hook_kind, func)?;
        Ok(CheckedTemplateBuilder {
            hook_kind: self.hook_kind,
            body_fn: Some(BodyFnSnapshot {
                name: func.name.clone(),
                sig,
                arity: func.params.len(),
                type_param_count: func
                    .type_params
                    .as_ref()
                    .map(|params| params.len())
                    .unwrap_or(0),
                body: func.body.clone(),
            }),
            captures: self.captures,
            _sig: PhantomData,
            _captures: PhantomData,
        })
    }

    /// Supply the complete capture set, advancing `CapturesState` to
    /// [`Present`]. An EMPTY clause is meaningful: it declares that the
    /// template captures nothing (Decision-95 "complete environment" — never
    /// inferred; C3-G4: config enters ONLY as declared captures).
    pub(in crate::compiler) fn captures(
        self,
        captures: CaptureClause,
    ) -> CheckedTemplateBuilder<SigState, Present> {
        CheckedTemplateBuilder {
            hook_kind: self.hook_kind,
            body_fn: self.body_fn,
            captures: Some(captures),
            _sig: PhantomData,
            _captures: PhantomData,
        }
    }
}

impl CheckedTemplateBuilder<Present, Present> {
    /// Validate the typed inputs and produce the [`CheckedTemplate`], or a
    /// named `ShapeError` — never a silent partial.
    ///
    /// Construction rejections:
    ///
    /// - the capture-family two, which REUSE the shipped `checked_body`
    ///   validator verbatim (no third sentence producer): a borrow-mode
    ///   capture — `[C0902]`; a duplicate capture name — `[C0907]`;
    /// - for a [`TemplateSig::PolymorphicArgs`] body, every pseudo-tuple
    ///   usage rejection of
    ///   [`validate_pseudo_tuple_uses`] (precise uncoded sentences with
    ///   positive twins; S5 owns C09xx minting from C0931+).
    ///
    /// Per-specialization checking against a frozen target (C3-G10) happens
    /// LATER, at the `template_specialization` seam — this is construction
    /// only.
    pub(in crate::compiler) fn finish(self) -> Result<CheckedTemplate> {
        // Typestate guarantees both were supplied; the `Option` is an
        // implementation detail of the phantom-typed transitions, never a
        // runtime completeness gate.
        let mut snapshot = self
            .body_fn
            .expect("Present SigState guarantees a body fn was supplied");
        let captures = self
            .captures
            .expect("Present CapturesState guarantees a capture set was supplied");

        validate_capture_clause(&captures)?;

        if let TemplateSig::PolymorphicArgs {
            type_param,
            args_param,
        } = &snapshot.sig
        {
            // The walker's `&mut` is traversal-uniformity with its rewrite
            // face only; the validate face never mutates (pseudo_tuple.rs
            // module docs).
            validate_pseudo_tuple_uses(&mut snapshot.body, args_param, type_param)?;
        }

        Ok(CheckedTemplate {
            hook_kind: self.hook_kind,
            body_fn: snapshot.name,
            sig: snapshot.sig,
            captures,
            arity: snapshot.arity,
            type_param_count: snapshot.type_param_count,
        })
    }
}

/// The explicit classification rule (no guessing):
///
/// - `type_params` None/empty → [`TemplateSig::Concrete`] (the C3-G4
///   degenerate case; the body was checked at definition under its own
///   signature).
/// - Exactly ONE plain (non-const) type param `T`, exactly one plain by-value
///   value param annotated bare-`T` (`Basic("T")` or a single-segment
///   `Reference`), and return annotation bare-`T` → the polymorphic form:
///   [`TemplateSig::PolymorphicArgs`] for a [`TemplateHookKind::Before`]
///   template, [`TemplateSig::PolymorphicResult`] for
///   [`TemplateHookKind::After`].
/// - Everything else → a named rejection (uncoded sentence + positive twin).
///
/// The variant derives FROM the hook kind, so "PolymorphicArgs offered to
/// After" (and the converse) is unrepresentable through this builder: no
/// public [`CheckedTemplate`] constructor exists that could pair a sig
/// variant with the wrong hook kind.
fn classify_template_sig(hook_kind: TemplateHookKind, func: &FunctionDef) -> Result<TemplateSig> {
    let type_params: &[TypeParam] = func.type_params.as_deref().unwrap_or(&[]);

    if type_params.is_empty() {
        return Ok(TemplateSig::Concrete(BodySignature::new(
            func.params.clone(),
            func.return_type.clone(),
        )));
    }

    let form = polymorphic_form(hook_kind);

    if type_params.len() > 1 {
        return Err(reject(format!(
            "template body fn `{}` declares {} type parameters; a polymorphic template body \
             declares exactly one plain type parameter (`{form}`), and a concrete template body \
             declares none",
            func.name,
            type_params.len(),
        )));
    }

    let type_param = &type_params[0];
    if type_param.is_const() {
        return Err(reject(format!(
            "template body fn `{}` declares a const generic parameter `{}`; a polymorphic \
             template body declares exactly one plain type parameter (`{form}`)",
            func.name,
            type_param.name(),
        )));
    }
    let type_param_name = type_param.name();

    if func.params.len() != 1 {
        return Err(reject(format!(
            "template body fn `{}` is generic over `{type_param_name}` but declares {} value \
             parameters; a polymorphic template body declares exactly one value parameter \
             annotated with its type parameter (`{form}`)",
            func.name,
            func.params.len(),
        )));
    }

    let param = &func.params[0];
    let param_name = match param.simple_name() {
        Some(name)
            if !param.is_const && !param.is_reference && !param.is_out
                && param.default_value.is_none() =>
        {
            name
        }
        _ => {
            return Err(reject(format!(
                "template body fn `{}` is generic over `{type_param_name}` but its value \
                 parameter is not a plain by-value identifier parameter; a polymorphic template \
                 body declares exactly one plain parameter annotated with its type parameter \
                 (`{form}`)",
                func.name,
            )));
        }
    };

    match &param.type_annotation {
        Some(annotation) if is_bare_type_param(annotation, type_param_name) => {}
        Some(annotation) => {
            return Err(reject(format!(
                "template body fn `{}` is generic over `{type_param_name}` but its value \
                 parameter is annotated `{}`; a polymorphic template body annotates its one \
                 parameter with exactly the type parameter (`{form}`)",
                func.name,
                annotation.to_type_string(),
            )));
        }
        None => {
            return Err(reject(format!(
                "template body fn `{}` is generic over `{type_param_name}` but its value \
                 parameter has no type annotation; a polymorphic template body annotates its one \
                 parameter with exactly the type parameter (`{form}`)",
                func.name,
            )));
        }
    }

    match &func.return_type {
        Some(annotation) if is_bare_type_param(annotation, type_param_name) => {}
        Some(annotation) => {
            return Err(reject(format!(
                "template body fn `{}` is generic over `{type_param_name}` but returns `{}`; a \
                 polymorphic template body returns exactly its type parameter (`{form}`)",
                func.name,
                annotation.to_type_string(),
            )));
        }
        None => {
            return Err(reject(format!(
                "template body fn `{}` is generic over `{type_param_name}` but declares no \
                 return type; a polymorphic template body returns exactly its type parameter \
                 (`{form}`)",
                func.name,
            )));
        }
    }

    Ok(match hook_kind {
        TemplateHookKind::Before => TemplateSig::PolymorphicArgs {
            type_param: type_param_name.to_string(),
            args_param: param_name.to_string(),
        },
        TemplateHookKind::After => TemplateSig::PolymorphicResult {
            type_param: type_param_name.to_string(),
            result_param: param_name.to_string(),
        },
    })
}

/// The accepted polymorphic spelling for each hook kind (the positive twin
/// every classification rejection carries).
fn polymorphic_form(hook_kind: TemplateHookKind) -> &'static str {
    match hook_kind {
        TemplateHookKind::Before => "fn t<Args>(args: Args) -> Args",
        TemplateHookKind::After => "fn t<R>(result: R) -> R",
    }
}

/// Bare-`T`: `Basic("T")` or a single-segment `Reference` naming `T` — the
/// same single-segment rule the monomorphization substituter uses
/// (`substitution.rs` soundness note: a qualified `mod::T` is never a type
/// parameter).
fn is_bare_type_param(annotation: &TypeAnnotation, type_param: &str) -> bool {
    match annotation {
        TypeAnnotation::Basic(name) => name == type_param,
        TypeAnnotation::Reference(path) => !path.is_qualified() && path.name() == type_param,
        _ => false,
    }
}

fn reject(message: String) -> ShapeError {
    ShapeError::SemanticError {
        message,
        location: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::span::Span;
    use shape_ast::ast::{CaptureEntry, CaptureMode};

    fn fn_def(src: &str) -> FunctionDef {
        shape_ast::parse_program(src)
            .expect("fixture parses")
            .items
            .into_iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(func, _) => Some(func),
                _ => None,
            })
            .expect("fixture has one function")
    }

    fn entry(mode: CaptureMode, name: &str) -> CaptureEntry {
        CaptureEntry {
            mode,
            name: name.to_string(),
            span: Span::default(),
            name_span: Span::default(),
        }
    }

    fn clause(entries: Vec<CaptureEntry>) -> CaptureClause {
        CaptureClause {
            entries,
            span: Span::default(),
        }
    }

    // HAPPY PATH: a Before template over the polymorphic form classifies as
    // the G9 pseudo-tuple binding.
    #[test]
    fn before_polymorphic_form_classifies_as_polymorphic_args() {
        let func = fn_def("fn my_before<Args>(args: Args) -> Args { return args }");
        let template = CheckedTemplateBuilder::new(TemplateHookKind::Before)
            .body_fn(&func)
            .expect("polymorphic before form classifies")
            .captures(clause(Vec::new()))
            .finish()
            .expect("well-formed inputs finish");

        assert_eq!(template.hook_kind(), TemplateHookKind::Before);
        assert_eq!(template.body_fn(), "my_before");
        assert_eq!(template.arity(), 1);
        assert_eq!(template.type_param_count(), 1);
        match template.sig() {
            TemplateSig::PolymorphicArgs {
                type_param,
                args_param,
            } => {
                assert_eq!(type_param, "Args");
                assert_eq!(args_param, "args");
            }
            other => panic!("expected PolymorphicArgs, got {other:?}"),
        }
    }

    // HAPPY PATH: an After template over the polymorphic form classifies as
    // the typed-result binding.
    #[test]
    fn after_polymorphic_form_classifies_as_polymorphic_result() {
        let func = fn_def("fn my_after<R>(result: R) -> R { return result }");
        let template = CheckedTemplateBuilder::new(TemplateHookKind::After)
            .body_fn(&func)
            .expect("polymorphic after form classifies")
            .captures(clause(Vec::new()))
            .finish()
            .expect("well-formed inputs finish");

        assert_eq!(template.hook_kind(), TemplateHookKind::After);
        assert_eq!(template.type_param_count(), 1);
        match template.sig() {
            TemplateSig::PolymorphicResult {
                type_param,
                result_param,
            } => {
                assert_eq!(type_param, "R");
                assert_eq!(result_param, "result");
            }
            other => panic!("expected PolymorphicResult, got {other:?}"),
        }
    }

    // HAPPY PATH: a concrete body is the G4 degenerate case, for BOTH hook
    // kinds.
    #[test]
    fn concrete_body_classifies_as_concrete_for_both_hook_kinds() {
        for hook_kind in [TemplateHookKind::Before, TemplateHookKind::After] {
            let func = fn_def("fn t(x: int) -> int { return x + 1 }");
            let template = CheckedTemplateBuilder::new(hook_kind)
                .body_fn(&func)
                .expect("concrete form classifies")
                .captures(clause(Vec::new()))
                .finish()
                .expect("well-formed inputs finish");

            assert_eq!(template.arity(), 1);
            assert_eq!(template.type_param_count(), 0);
            match template.sig() {
                TemplateSig::Concrete(signature) => {
                    assert_eq!(signature.params().len(), 1);
                    assert!(signature.return_type().is_some());
                }
                other => panic!("expected Concrete, got {other:?}"),
            }
        }
    }

    // The typestate is order-independent: captures-then-body_fn reaches the
    // same `<Present, Present>` finish() as body_fn-then-captures.
    #[test]
    fn typestate_transitions_are_order_independent() {
        let func = fn_def("fn t<Args>(args: Args) -> Args { return args }");
        let template = CheckedTemplateBuilder::new(TemplateHookKind::Before)
            .captures(clause(vec![entry(CaptureMode::Move, "cfg")]))
            .body_fn(&func)
            .expect("classification is order-independent")
            .finish()
            .expect("order-independent supply finishes");
        assert_eq!(template.captures().len(), 1);
    }

    // NEGATIVE (reused validator, no third sentence producer): borrow-mode
    // capture -> [C0902].
    #[test]
    fn borrow_mode_capture_is_rejected_c0902_via_reused_validator() {
        let func = fn_def("fn t<Args>(args: Args) -> Args { return args }");
        let err = CheckedTemplateBuilder::new(TemplateHookKind::Before)
            .body_fn(&func)
            .expect("classification passes")
            .captures(clause(vec![entry(CaptureMode::SharedBorrow, "handle")]))
            .finish()
            .expect_err("borrow-mode capture must be rejected");
        assert!(
            err.to_string().contains("[C0902]"),
            "expected [C0902], got: {err}"
        );
    }

    // NEGATIVE (reused validator): duplicate capture name -> [C0907].
    #[test]
    fn duplicate_capture_is_rejected_c0907_via_reused_validator() {
        let func = fn_def("fn t<Args>(args: Args) -> Args { return args }");
        let err = CheckedTemplateBuilder::new(TemplateHookKind::Before)
            .body_fn(&func)
            .expect("classification passes")
            .captures(clause(vec![
                entry(CaptureMode::Move, "dup"),
                entry(CaptureMode::Share, "dup"),
            ]))
            .finish()
            .expect_err("duplicate capture name must be rejected");
        assert!(
            err.to_string().contains("[C0907]"),
            "expected [C0907], got: {err}"
        );
    }

    fn expect_classification_reject(hook_kind: TemplateHookKind, src: &str, needle: &str) {
        let func = fn_def(src);
        let err = CheckedTemplateBuilder::new(hook_kind)
            .body_fn(&func)
            .err()
            .unwrap_or_else(|| panic!("expected classification rejection for {src:?}"));
        assert!(
            err.to_string().contains(needle),
            "expected rejection containing {needle:?}, got: {err}"
        );
    }

    // NEGATIVE classification: two type parameters.
    #[test]
    fn two_type_params_are_rejected() {
        expect_classification_reject(
            TemplateHookKind::Before,
            "fn t<A, B>(x: A) -> A { return x }",
            "declares 2 type parameters",
        );
    }

    // NEGATIVE classification: a const generic parameter.
    #[test]
    fn const_type_param_is_rejected() {
        expect_classification_reject(
            TemplateHookKind::Before,
            "fn t<const N: int>(x: int) -> int { return x }",
            "const generic parameter",
        );
    }

    // NEGATIVE classification: the type parameter mixed with concrete params.
    #[test]
    fn generic_with_extra_value_param_is_rejected() {
        expect_classification_reject(
            TemplateHookKind::Before,
            "fn t<Args>(args: Args, n: int) -> Args { return args }",
            "declares 2 value parameters",
        );
    }

    // NEGATIVE classification: the one parameter is not annotated bare-T.
    #[test]
    fn generic_param_not_bare_t_is_rejected() {
        expect_classification_reject(
            TemplateHookKind::Before,
            "fn t<Args>(args: Array<Args>) -> Args { return args[0] }",
            "annotates its one parameter with exactly the type parameter",
        );
    }

    // NEGATIVE classification: missing return annotation on the polymorphic
    // form.
    #[test]
    fn generic_missing_return_is_rejected() {
        expect_classification_reject(
            TemplateHookKind::Before,
            "fn t<Args>(args: Args) { return args }",
            "declares no return type",
        );
    }

    // NEGATIVE classification: non-T return on the polymorphic form.
    #[test]
    fn generic_non_t_return_is_rejected() {
        expect_classification_reject(
            TemplateHookKind::After,
            "fn t<R>(result: R) -> int { return 0 }",
            "returns `int`",
        );
    }

    // finish() runs the G9 pseudo-tuple walker for PolymorphicArgs bodies.
    #[test]
    fn finish_runs_pseudo_tuple_validation_for_polymorphic_args() {
        let func = fn_def(
            r#"
fn t<Args>(args: Args) -> Args {
    let i = 0
    args[i] = 1
    return args
}
"#,
        );
        let err = CheckedTemplateBuilder::new(TemplateHookKind::Before)
            .body_fn(&func)
            .expect("classification passes")
            .captures(clause(Vec::new()))
            .finish()
            .expect_err("a non-constant pseudo-tuple index must be rejected at finish()");
        assert!(
            err.to_string().contains("compile-time-constant index"),
            "expected the pseudo-tuple index rejection, got: {err}"
        );
    }

    // finish() does NOT pseudo-tuple-check an After polymorphic body: the
    // `result` param is an ordinary value (bare use is legal).
    #[test]
    fn after_polymorphic_body_is_not_pseudo_tuple_checked() {
        let func = fn_def(
            r#"
fn t<R>(result: R) -> R {
    let x = result
    return x
}
"#,
        );
        CheckedTemplateBuilder::new(TemplateHookKind::After)
            .body_fn(&func)
            .expect("classification passes")
            .captures(clause(Vec::new()))
            .finish()
            .expect("an after body uses its result param as an ordinary value");
    }

    // finish() does NOT pseudo-tuple-check a concrete body: its params are
    // ordinary declared values.
    #[test]
    fn concrete_body_is_not_pseudo_tuple_checked() {
        let func = fn_def("fn t(args: int) -> int { let x = args\n return x }");
        CheckedTemplateBuilder::new(TemplateHookKind::Before)
            .body_fn(&func)
            .expect("classification passes")
            .captures(clause(Vec::new()))
            .finish()
            .expect("a concrete body uses its params as ordinary values");
    }

    // COMPILE-TIME GUARANTEE (documented, not runtime-testable): `finish()` is
    // implemented only for `CheckedTemplateBuilder<Present, Present>`, so
    // `CheckedTemplateBuilder::new(kind).finish()` and a single-axis-supplied
    // builder do NOT compile. That unrepresentability IS the "never a silent
    // partial" guarantee — the same doc-note pin as checked_body.rs:431-436.
}
