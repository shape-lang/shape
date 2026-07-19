//! ADR-009 E1 #17 (slice 1) — the public typed builder for a checked generated
//! body: `CheckedBodyBuilder<SigState, CapturesState>` + `finish()`.
//!
//! This discharges the user-ratified C2-D1 amendment (issue #13): C2 shipped the
//! installation VALIDATOR (`crate::compiler::checked_body` — the atomic
//! `InstallTransaction` + the §4.2 ten-check battery); this slice ships the
//! public CONSTRUCTION surface that its consumers — E1's typed rewrite
//! directives (slices 3/4/5) — build a generated body through. The two are
//! distinct modules on purpose: `comptime_fragments::checked_body` (here) is the
//! construction chokepoint; `compiler::checked_body` is the install chokepoint.
//!
//! # The construction/install split (binding invariant — do not collapse)
//!
//! `finish()` is the CONSTRUCTION side only. It validates the TYPED inputs and
//! returns a provenance-ready [`CheckedBody`] carrier or a named `ShapeError` —
//! never a silent partial. It does NOT install, publish, stamp, or reserve; it
//! holds no `&mut BytecodeCompiler`. This mirrors the shipped `CheckedItem`
//! pattern ("provenance-READY, not yet reserved" — see the sibling module docs):
//! a comptime builtin has no `&mut` compiler access, so the atomic publish
//! happens later, at the directive consumer, through the ALREADY-open C2
//! `InstallTransaction` + battery.
//!
//! The Decision-95 "checks and installs atomically" property is therefore
//! discharged BY COMPOSITION, not by this module alone:
//!
//! ```text
//!   finish()                      -> CheckedBody         (construction-side, here)
//!   consumer + driver check seq   -> atomic install       (C2 seam, compiler::checked_body)
//! ```
//!
//! **Every consumer (slices 3-5) MUST route through BOTH — `finish()` to obtain
//! the carrier, THEN the driver's shared check sequence / C2 transaction to
//! publish it — never either alone.** `finish()` alone gives a validated but
//! un-installed carrier; the C2 transaction alone would bypass the typed
//! construction guarantees this builder exists to enforce.
//!
//! # No source-string escape hatch
//!
//! There is deliberately NO `from_source(&str)` / string / JSON constructor
//! anywhere on this surface — that reparse protocol is exactly what E1 exists to
//! delete. The only way to obtain a [`CheckedBody`] is through the typestate
//! builder with typed AST inputs.
//!
//! # Typestate: "never a silent partial" is a TYPE-SYSTEM guarantee
//!
//! `finish()` is implemented ONLY for `CheckedBodyBuilder<Present, Present>`, so
//! it is unrepresentable to finish a builder whose signature or capture set was
//! never supplied — the Rust type system rejects it at compile time, the same
//! discipline as `ProofGap`'s private constructor. No runtime "is it complete?"
//! check, and no partial `CheckedBody` can exist.
//!
//! The typestate parameters `SigState`/`CapturesState` track SUPPLIED-NESS only;
//! they are named distinctly from Decision-95's SEMANTIC `CheckedBody<Sig,
//! Captures>` type parameters (the Shape comptime-type face, which lands with the
//! Decision-95 Shape staging surface in a later E-track/C3 slice). The concrete
//! signature and capture pack are carried here as ordinary data.

use std::marker::PhantomData;

use shape_ast::ast::{CaptureClause, FunctionParameter, Statement, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};

/// Typestate marker: the axis has not been supplied yet.
pub(in crate::compiler) struct Missing;
/// Typestate marker: the axis has been supplied.
pub(in crate::compiler) struct Present;

/// The signature a [`CheckedBody`] matches: its positional parameters and return
/// type, straight from the AST the directive consumer already holds (the target
/// `FunctionDef`'s `params`/`return_type`). Slice 1 carries the signature; the
/// install-time battery (C2) is what type-checks the body against it.
#[derive(Clone)]
pub(in crate::compiler) struct BodySignature {
    params: Vec<FunctionParameter>,
    return_type: Option<TypeAnnotation>,
}

impl BodySignature {
    /// Carry a signature built from typed AST pieces. No string ever
    /// participates.
    pub(in crate::compiler) fn new(
        params: Vec<FunctionParameter>,
        return_type: Option<TypeAnnotation>,
    ) -> Self {
        Self {
            params,
            return_type,
        }
    }

    /// The positional parameters.
    pub(in crate::compiler) fn params(&self) -> &[FunctionParameter] {
        &self.params
    }

    /// The declared return type, if any.
    pub(in crate::compiler) fn return_type(&self) -> Option<&TypeAnnotation> {
        self.return_type.as_ref()
    }
}

/// A comptime-generated callable body, checked at CONSTRUCTION: it matches one
/// signature, carries a complete (explicitly-declared, never-inferred) capture
/// set, and holds a real AST body — never a reparsed source/JSON string. Built
/// ONLY through [`CheckedBodyBuilder::finish`]; the private fields and the absent
/// public constructor mean a caller cannot assemble one from unchecked inputs by
/// skipping the builder (the same discipline as the sibling `CheckedItem` /
/// `CheckedModule` carriers).
///
/// # Provenance-ready, not installed (see the module docs)
///
/// This carrier is the CONSTRUCTION-side result. It is not stamped, reserved, or
/// published. The atomic install is the consumer's job, through the C2
/// `InstallTransaction` — `finish()` and the C2 seam COMPOSE; a consumer must use
/// both.
pub(in crate::compiler) struct CheckedBody {
    signature: BodySignature,
    captures: CaptureClause,
    body: Vec<Statement>,
}

impl CheckedBody {
    /// The signature this body matches.
    pub(in crate::compiler) fn signature(&self) -> &BodySignature {
        &self.signature
    }

    /// The complete, explicitly-declared capture set.
    pub(in crate::compiler) fn captures(&self) -> &CaptureClause {
        &self.captures
    }

    /// The generated body statements.
    pub(in crate::compiler) fn body(&self) -> &[Statement] {
        &self.body
    }

    /// Consume into the body statements for the consumer's install sequence.
    pub(in crate::compiler) fn into_body(self) -> Vec<Statement> {
        self.body
    }
}

/// The typed builder for a [`CheckedBody`]. Start with [`Self::new`], supply the
/// signature ([`Self::signature`]) and the capture set ([`Self::captures`]) in
/// any order, optionally set the body ([`Self::body`]), then [`finish`] — which
/// exists ONLY once BOTH `SigState` and `CapturesState` are [`Present`].
///
/// [`finish`]: CheckedBodyBuilder::finish
pub(in crate::compiler) struct CheckedBodyBuilder<SigState, CapturesState> {
    signature: Option<BodySignature>,
    captures: Option<CaptureClause>,
    body: Vec<Statement>,
    _sig: PhantomData<SigState>,
    _captures: PhantomData<CapturesState>,
}

impl CheckedBodyBuilder<Missing, Missing> {
    /// A fresh builder — no signature, no capture set, an empty body.
    pub(in crate::compiler) fn new() -> Self {
        Self {
            signature: None,
            captures: None,
            body: Vec::new(),
            _sig: PhantomData,
            _captures: PhantomData,
        }
    }
}

impl<SigState, CapturesState> CheckedBodyBuilder<SigState, CapturesState> {
    /// Supply the signature, advancing `SigState` to [`Present`].
    pub(in crate::compiler) fn signature(
        self,
        signature: BodySignature,
    ) -> CheckedBodyBuilder<Present, CapturesState> {
        CheckedBodyBuilder {
            signature: Some(signature),
            captures: self.captures,
            body: self.body,
            _sig: PhantomData,
            _captures: PhantomData,
        }
    }

    /// Supply the complete capture set, advancing `CapturesState` to
    /// [`Present`]. An EMPTY clause is meaningful: it declares that the body
    /// captures nothing (Decision-95 "complete environment" — never inferred).
    pub(in crate::compiler) fn captures(
        self,
        captures: CaptureClause,
    ) -> CheckedBodyBuilder<SigState, Present> {
        CheckedBodyBuilder {
            signature: self.signature,
            captures: Some(captures),
            body: self.body,
            _sig: PhantomData,
            _captures: PhantomData,
        }
    }

    /// Set the generated body statements (typed AST, never a source string).
    /// Available in any typestate; an unset or empty body is a `finish()`
    /// rejection, not a type error.
    pub(in crate::compiler) fn body(mut self, body: Vec<Statement>) -> Self {
        self.body = body;
        self
    }
}

impl CheckedBodyBuilder<Present, Present> {
    /// Validate the typed inputs and produce the provenance-ready
    /// [`CheckedBody`], or a named `ShapeError` — never a silent partial.
    ///
    /// This is the CONSTRUCTION chokepoint ONLY (see the module docs): it does
    /// not install, stamp, reserve, or publish. Construction rejections:
    ///
    /// - a borrow-mode capture (`&` / `&mut`) — `[C0902]`, reserved until Shape
    ///   has a closure-region story (the authoritative capture-family code);
    /// - a duplicate capture name — `[C0907]` (the authoritative code the
    ///   capture planner uses; slice 1 checks the name-level subset available at
    ///   construction, before slot resolution);
    /// - an empty body — a checked generated body must contain at least one
    ///   statement.
    ///
    /// The install-time §4.2 battery (type/effect/ownership/borrow/lifetime/
    /// suspension/Send/cleanup/Drop/async-drop) runs LATER, at the C2 seam the
    /// consumer routes this carrier through.
    pub(in crate::compiler) fn finish(self) -> Result<CheckedBody> {
        // Typestate guarantees both were supplied; the `Option` is an
        // implementation detail of the phantom-typed transitions, never a
        // runtime completeness gate.
        let signature = self
            .signature
            .expect("Present SigState guarantees a signature was supplied");
        let captures = self
            .captures
            .expect("Present CapturesState guarantees a capture set was supplied");

        validate_capture_clause(&captures)?;

        if self.body.is_empty() {
            return Err(ShapeError::RuntimeError {
                message: "a checked generated body must contain at least one statement; an \
                          empty body cannot match a signature"
                    .to_string(),
                location: None,
            });
        }

        Ok(CheckedBody {
            signature,
            captures,
            body: self.body,
        })
    }
}

/// Construction-time validation of the declared capture set — the subset
/// checkable WITHOUT scope/slot resolution (that is the capture planner's
/// install-time job). Reuses the authoritative capture-family codes (D4: reuse,
/// do not mint parallel codes).
fn validate_capture_clause(clause: &CaptureClause) -> Result<()> {
    let mut seen: Vec<&str> = Vec::with_capacity(clause.entries.len());
    for entry in &clause.entries {
        if entry.mode.is_borrow() {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "[C0902] capture '{} {}' uses a borrow mode; a borrow that escapes into a \
                     generated body has no lifetime to check and is reserved until Shape has a \
                     closure-region story",
                    entry.mode.spelling(),
                    entry.name
                ),
                location: None,
            });
        }
        if seen.contains(&entry.name.as_str()) {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "[C0907] duplicate capture declaration for '{}'; each captured binding may \
                     be declared at most once",
                    entry.name
                ),
                location: None,
            });
        }
        seen.push(entry.name.as_str());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::span::Span;
    use shape_ast::ast::{CaptureEntry, CaptureMode};

    fn body_of(src: &str) -> Vec<Statement> {
        shape_ast::parse_program(src)
            .expect("fixture parses")
            .items
            .into_iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(func, _) => Some(func.body),
                _ => None,
            })
            .expect("fixture has one function")
    }

    fn sig() -> BodySignature {
        BodySignature::new(Vec::new(), Some(TypeAnnotation::Basic("int".to_string())))
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

    // HAPPY PATH: a fully-supplied builder finishes into a carrier that reflects
    // its typed inputs.
    #[test]
    fn finish_produces_a_carrier_reflecting_the_typed_inputs() {
        let checked = CheckedBodyBuilder::new()
            .signature(sig())
            .captures(clause(vec![entry(CaptureMode::Move, "cfg")]))
            .body(body_of("fn f() -> int { 1 }"))
            .finish()
            .expect("well-formed inputs finish");

        assert_eq!(checked.captures().len(), 1);
        assert!(checked.signature().return_type().is_some());
        assert_eq!(checked.body().len(), 1);
        let body = checked.into_body();
        assert_eq!(body.len(), 1);
    }

    // An EMPTY capture clause is a valid "captures nothing" declaration, not a
    // rejection (Decision-95: complete environment, explicitly empty).
    #[test]
    fn empty_capture_clause_is_a_valid_complete_environment() {
        let checked = CheckedBodyBuilder::new()
            .captures(clause(Vec::new()))
            .signature(sig())
            .body(body_of("fn f() -> int { 7 }"))
            .finish()
            .expect("empty capture set is complete, not partial");
        assert!(checked.captures().is_empty());
    }

    // NEGATIVE: borrow-mode capture -> [C0902].
    #[test]
    fn borrow_mode_capture_is_rejected_c0902() {
        for mode in [CaptureMode::SharedBorrow, CaptureMode::ExclusiveBorrow] {
            let err = CheckedBodyBuilder::new()
                .signature(sig())
                .captures(clause(vec![entry(mode, "handle")]))
                .body(body_of("fn f() -> int { 1 }"))
                .finish()
                .expect_err("borrow-mode capture must be rejected");
            assert!(
                err.to_string().contains("[C0902]"),
                "expected [C0902], got: {err}"
            );
        }
    }

    // NEGATIVE: duplicate capture name -> [C0907].
    #[test]
    fn duplicate_capture_name_is_rejected_c0907() {
        let err = CheckedBodyBuilder::new()
            .signature(sig())
            .captures(clause(vec![
                entry(CaptureMode::Move, "dup"),
                entry(CaptureMode::Share, "dup"),
            ]))
            .body(body_of("fn f() -> int { 1 }"))
            .finish()
            .expect_err("duplicate capture name must be rejected");
        assert!(
            err.to_string().contains("[C0907]"),
            "expected [C0907], got: {err}"
        );
    }

    // NEGATIVE: empty body (never set) -> named rejection.
    #[test]
    fn empty_body_is_rejected() {
        let err = CheckedBodyBuilder::new()
            .signature(sig())
            .captures(clause(Vec::new()))
            .finish()
            .expect_err("empty body must be rejected");
        assert!(
            err.to_string().contains("empty body"),
            "expected an empty-body rejection, got: {err}"
        );
    }

    // The typestate is order-independent: captures-then-signature reaches the
    // same `<Present, Present>` finish() as signature-then-captures.
    #[test]
    fn typestate_transitions_are_order_independent() {
        let checked = CheckedBodyBuilder::new()
            .captures(clause(Vec::new()))
            .signature(sig())
            .body(body_of("fn f() -> int { 3 }"))
            .finish()
            .expect("order-independent supply finishes");
        assert_eq!(checked.body().len(), 1);
    }

    // COMPILE-TIME GUARANTEE (documented, not runtime-testable): `finish()` is
    // implemented only for `CheckedBodyBuilder<Present, Present>`, so
    // `CheckedBodyBuilder::new().finish()` and a single-axis-supplied builder do
    // NOT compile. That unrepresentability IS the "never a silent partial"
    // guarantee; a runtime test cannot exercise a non-compiling call, so it is
    // pinned here as a doc note per the slice-1 review conditions.
}
